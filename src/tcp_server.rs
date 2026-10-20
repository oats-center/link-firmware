use defmt::{info, warn};
use embassy_net::{tcp::TcpSocket, Stack};
use embassy_time::{Timer};

use crate::ethernet::TCP_PORT;
use crate::protocol::{
    FrameHeader, MessageType, HEADER_LEN, HELLO_PAYLOAD, MAX_PAYLOAD_LEN,
};

const RX_TCP_BUFFER_BYTES: usize = 1024;
const TX_TCP_BUFFER_BYTES: usize = 1536;
const ERROR_NOT_IMPLEMENTED: &[u8] = b"NOT_IMPLEMENTED";

// One listener and one active host connection at a time
#[embassy_executor::task]
pub async fn tcp_server_task(stack: Stack<'static>) -> ! {
    let mut tcp_rx = [0u8; RX_TCP_BUFFER_BYTES];
    let mut tcp_tx = [0u8; TX_TCP_BUFFER_BYTES];
    // This is the largest accepted application payload, not a TCP buffer
    let mut payload = [0u8; MAX_PAYLOAD_LEN];

    stack.wait_config_up().await;

    loop {
        info!("AVDQ: waiting for W5500 Ethernet link");
        stack.wait_link_up().await;
        info!("AVDQ: listening on TCP port 7600");

        {
            let mut socket = TcpSocket::new(stack, &mut tcp_rx, &mut tcp_tx);

            match socket.accept(TCP_PORT).await {
                Ok(()) => info!("AVDQ: host connected"),
                Err(_) => {
                    warn!("AVDQ: accept failed");
                    continue;
                }
            }

            // Treat any I/O/protocol failure as a connection-level failure
            if handle_connection(&mut socket, &mut payload).await.is_err() {
                warn!("AVDQ: closed connection after I/O or protocol error");
            } else {
                info!("AVDQ: host disconnected");
            }
            // `socket` is dropped here, freeing the single TCP socket entry
        }

        Timer::after_millis(100).await;
    }
}

// Reads exactly a frame component regardless of TCP segmentation
// Ok(false) means EOF. Callers close the session even if EOF was mid-frame
async fn read_fully(socket: &mut TcpSocket<'_>, target: &mut [u8]) -> Result<bool, ()> {
    let mut completed = 0;
    while completed < target.len() {
        match socket.read(&mut target[completed..]).await {
            Ok(0) => return Ok(false),
            Ok(received) => completed += received,
            Err(_) => return Err(()),
        }
    }
    Ok(true)
}

async fn write_fully(
    socket: &mut TcpSocket<'_>,
    data: &[u8],
) -> Result<(), ()> {
    let mut sent = 0;

    while sent < data.len() {
        match socket.write(&data[sent..]).await {
            Ok(0) => return Err(()),
            Ok(n) => sent += n,
            Err(_) => return Err(()),
        }
    }

    Ok(())
}

// Two writes do not imply two TCP packets. Host must parse the length field and perform exact-length reads
async fn send_frame(
    socket: &mut TcpSocket<'_>,
    kind: MessageType,
    sequence: u32,
    payload: &[u8],
) -> Result<(), ()> {
    let header = FrameHeader::new(kind, sequence, payload.len())
        .map_err(|_| ())?
        .encode();

    write_fully(socket, &header).await?;

    if !payload.is_empty() {
        write_fully(socket, payload).await?;
    }
    Ok(())
}

async fn handle_connection(
    socket: &mut TcpSocket<'_>,
    payload: &mut [u8; MAX_PAYLOAD_LEN],
) -> Result<(), ()> {
    // The board always initiates the application protocol after TCP connect
    // HELLO has sequence 0, as it is not a reply to a host command
    send_frame(socket, MessageType::Hello, 0, HELLO_PAYLOAD).await?;

    loop {
        let mut raw_header = [0u8; HEADER_LEN];
        if !read_fully(socket, &mut raw_header).await? {
            return Ok(()); // clean EOF (or incomplete frame before EOF)
        }

        let header = FrameHeader::decode(&raw_header).map_err(|_| {
            warn!("AVDQ: malformed header, incorrect version, or large payload");
        })?;

        let len = header.payload_len as usize;
        if len > 0 && !read_fully(socket, &mut payload[..len]).await? {
            // EOF during payload. Never execute a partially read command
            return Err(());
        }

        match MessageType::from_wire(header.message_type) {
            Some(MessageType::Ping) => {
                send_frame(socket, MessageType::Pong, header.sequence, &payload[..len]).await?;
            }
            _ => {
                // TODO: Start/Stop/Configure/Data/unknown are NOT implemented yet
                send_frame(
                    socket,
                    MessageType::Error,
                    header.sequence,
                    ERROR_NOT_IMPLEMENTED,
                )
                .await?;
            }
        }
    }
}
