use defmt::{info, warn};
use embassy_futures::select::{select, Either};
use embassy_net::{tcp::TcpSocket, Stack};
use embassy_time::{Instant, Timer};

use crate::acquisition;
use crate::ethernet::TCP_PORT;
use crate::protocol::{
    encode_data_payload, FrameHeader, MessageType, DATA_PAYLOAD_LEN, HEADER_LEN,
    HELLO_PAYLOAD, MAX_PAYLOAD_LEN,
};
use crate::sample_block::SampleBlock;

const RX_TCP_BUFFER_BYTES: usize = 1024;
const TX_TCP_BUFFER_BYTES: usize = 4096;
const INPUT_CHUNK_BYTES: usize = 256;
const ERROR_NOT_IMPLEMENTED: &[u8] = b"NOT_IMPLEMENTED";
const ERROR_INVALID_COMMAND: &[u8] = b"INVALID_COMMAND";
const ERROR_ALREADY_STREAMING: &[u8] = b"ALREADY_STREAMING";
const SIM_STARTED: &[u8] = b"SIM_STARTED"; // temporary milestone-4 ACK

// One listener and one active host connection at a time
#[embassy_executor::task]
pub async fn tcp_server_task(stack: Stack<'static>) -> ! {
    let mut tcp_rx = [0u8; RX_TCP_BUFFER_BYTES];
    let mut tcp_tx = [0u8; TX_TCP_BUFFER_BYTES];

    stack.wait_config_up().await;

    loop {
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
            if handle_connection(&mut socket).await.is_err() {
                warn!("AVDQ: closed connection after I/O or protocol error");
            } else {
                info!("AVDQ: host disconnected");
            }
            // `socket` is dropped here, freeing the single TCP socket entry
        }

        Timer::after_millis(100).await;
    }
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

    let total = HEADER_LEN + payload.len();
    let mut frame = [0u8; HEADER_LEN + MAX_PAYLOAD_LEN];
    frame[..HEADER_LEN].copy_from_slice(&header);
    frame[HEADER_LEN..total].copy_from_slice(payload);
    write_fully(socket, &frame[..total]).await
}

async fn send_data(
    socket: &mut TcpSocket<'_>,
    block: &SampleBlock,
    sequence: u32,
) -> Result<(), ()> {
    let mut encoded = [0u8; DATA_PAYLOAD_LEN];
    encode_data_payload(block, acquisition::dropped_blocks(), &mut encoded);
    send_frame(socket, MessageType::Data, sequence, &encoded).await
}

// A bounded frame parser, accepting any TCP segmentation and concatenation.
// A malicious or malformed length is rejected before buffering its payload.
struct FrameParser {
    buffer: [u8; HEADER_LEN + MAX_PAYLOAD_LEN],
    used: usize,
    expected: usize,
    header: Option<FrameHeader>,
}

impl FrameParser {
    fn new() -> Self {
        Self {
            buffer: [0u8; HEADER_LEN + MAX_PAYLOAD_LEN],
            used: 0,
            expected: HEADER_LEN,
            header: None,
        }
    }

    fn push_byte(&mut self, byte: u8) -> Result<Option<FrameHeader>, ()> {
        if self.used >= self.buffer.len() {
            return Err(());
        }
        self.buffer[self.used] = byte;
        self.used += 1;

        if self.used == HEADER_LEN {
            let mut header_bytes = [0u8; HEADER_LEN];
            header_bytes.copy_from_slice(&self.buffer[..HEADER_LEN]);
            let header = FrameHeader::decode(&header_bytes).map_err(|_| ())?;
            self.expected = HEADER_LEN + header.payload_len as usize;
            self.header = Some(header);
        }

        if self.used == self.expected {
            Ok(self.header)
        } else {
            Ok(None)
        }
    }

    fn payload(&self) -> &[u8] {
        &self.buffer[HEADER_LEN..self.used]
    }

    fn reset(&mut self) {
        self.used = 0;
        self.expected = HEADER_LEN;
        self.header = None;
    }
}

async fn handle_command(
    socket: &mut TcpSocket<'_>,
    header: FrameHeader,
    payload: &[u8],
    streaming: &mut bool,
    data_sequence: &mut u32,
) -> Result<(), ()> {
    match MessageType::from_wire(header.message_type) {
        Some(MessageType::Ping) => {
            send_frame(socket, MessageType::Pong, header.sequence, payload).await?;
        }
        Some(MessageType::Start) if payload.is_empty() => {
            if *streaming {
                send_frame(socket, MessageType::Error, header.sequence, ERROR_ALREADY_STREAMING).await?;
            } else {
                acquisition::begin_session();
                *data_sequence = 1;
                *streaming = true;
                // Temporary ACK; this will become a typed status in milestone 5.
                send_frame(socket, MessageType::Status, header.sequence, SIM_STARTED).await?;
            }
        }
        Some(MessageType::Stop) if payload.is_empty() => {
            acquisition::end_session();
            *streaming = false;
            send_frame(socket, MessageType::Stopped, header.sequence, &[]).await?;
        }
        Some(MessageType::Configure) => {
            send_frame(socket, MessageType::Error, header.sequence, ERROR_NOT_IMPLEMENTED).await?;
        }
        _ => {
            send_frame(socket, MessageType::Error, header.sequence, ERROR_INVALID_COMMAND).await?;
        }
    }
    Ok(())
}

async fn handle_connection(socket: &mut TcpSocket<'_>) -> Result<(), ()> {
    acquisition::end_session();
    send_frame(socket, MessageType::Hello, 0, HELLO_PAYLOAD).await?;
    let result = connection_loop(socket).await;
    // Even on I/O error or EOF, stop the producer and clear queued blocks.
    acquisition::end_session();
    result
}

async fn connection_loop(socket: &mut TcpSocket<'_>) -> Result<(), ()> {
    let mut parser = FrameParser::new();
    let mut input_chunk = [0u8; INPUT_CHUNK_BYTES];
    let mut streaming = false;
    let mut data_sequence: u32 = 1;
    let mut stats_start = Instant::now();
    let mut frames_sent_this_second: u32 = 0;
    let mut slowest_send_ms: u64 = 0;

    loop {
        // A single writer serializes PONG/STATUS/STOPPED and DATA frames.
        // `select` permits processing incoming control messages while waiting
        // for simulated blocks. The pending future is simply cancelled.
        match select(socket.read(&mut input_chunk), acquisition::SAMPLE_QUEUE.receive()).await {
            Either::First(Ok(0)) => return Ok(()),
            Either::First(Err(_)) => return Err(()),
            Either::First(Ok(n)) => {
                for &byte in &input_chunk[..n] {
                    if let Some(header) = parser.push_byte(byte).map_err(|_| {
                        warn!("AVDQ: malformed or oversized incoming frame");
                    })? {
                        handle_command(socket, header, parser.payload(), &mut streaming, &mut data_sequence).await?;
                        parser.reset();
                    }
                }
            }
            Either::Second(block) => {
                if streaming {
                    let send_start = Instant::now();
                    send_data(socket, &block, data_sequence).await?;
                    slowest_send_ms = slowest_send_ms.max(send_start.elapsed().as_millis());
                    data_sequence = data_sequence.wrapping_add(1);
                    frames_sent_this_second += 1;

                    // One diagnostic per second; avoid log traffic per sample.
                    // send time measures enqueueing into TCP, not remote ACK.
                    if stats_start.elapsed().as_millis() >= 1_000 {
                        info!(
                            "AVDQ tx/s={=u32} max_enqueue_ms={=u64} queued={=usize} dropped={=u32}",
                            frames_sent_this_second,
                            slowest_send_ms,
                            acquisition::SAMPLE_QUEUE.len(),
                            acquisition::dropped_blocks(),
                        );
                        frames_sent_this_second = 0;
                        slowest_send_ms = 0;
                        stats_start = Instant::now();
                    }
                }
            }
        }
    }
}