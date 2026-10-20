use core::fmt::{self, Write};

use defmt::info;
use embassy_stm32::mode::Async;
use embassy_stm32::usart::{Error, Uart, UartTx};

use crate::ads131m::Ads131m;
use crate::max31856::Max31856;
use crate::sdi12::{self, Sdi12Bus};
use sdi12::{Sdi12Error, SerialCommand};

pub async fn receive(
    usart: &mut Uart<'_, Async>,
    sdi12: &mut impl Sdi12Bus,
    thermocouples: &mut Max31856<'_>,
    adc: &mut Ads131m<'_>,
) -> Result<(), Error> {
    let mut rx_buf = [0u8; 64];
    let mut tx_buf = [0u8; 256];

    let (mut usart_tx, usart_rx) = usart.split_ref();

    let bytes_read = usart_rx.read_until_idle(&mut rx_buf).await?;

    if let Ok(cmd_str) = core::str::from_utf8(&rx_buf[..bytes_read]) {
        match get_response(sdi12, thermocouples, adc, cmd_str, &mut tx_buf).await {
            Ok(size) if size > 0 => usart_tx.write(&tx_buf[..size]).await,
            Ok(_) => handle_error(&mut usart_tx, Sdi12Error::InvalidSDI12Command).await,
            Err(e) => handle_error(&mut usart_tx, e).await,
        }
    } else {
        handle_error(&mut usart_tx, Sdi12Error::InvalidSDI12Command).await
    }
}

pub async fn get_response(
    sdi12: &mut impl Sdi12Bus,
    thermocouples: &mut Max31856<'_>,
    adc: &mut Ads131m<'_>,
    cmd_str: &str,
    output: &mut [u8],
) -> Result<usize, Sdi12Error> {
    let parsed_cmd: SerialCommand = parse_cmd(cmd_str)?;

    let bytes_to_send = match parsed_cmd {
        SerialCommand::Ping => {
            let msg = b"PONG\r\n";
            output[..msg.len()].copy_from_slice(msg);
            msg.len()
        }

        SerialCommand::Scan {
            start_addr,
            end_addr,
        } => {
            if start_addr.is_whitespace() || end_addr.is_whitespace() {
                let msg = b"SCAN <START_ADDR>,<END_ADDR>\r\n";
                output[..msg.len()].copy_from_slice(msg);
                return Ok(msg.len());
            }

            if !cmd_str.contains('!') {
                return Err(Sdi12Error::InvalidSDI12Command);
            }

            let mut addr = start_addr as u8;
            let mut index = 0;

            loop {
                let msg = [addr, b'!'];

                // TODO: possible out of bounds error if index > output.len()?
                let bytes_read = match sdi12.query_device(&msg, &mut output[index..]).await {
                    Ok(b) => b,
                    Err(e) => match e {
                        Sdi12Error::Timeout => 0,
                        _ => return Err(e),
                    },
                };

                index += bytes_read;

                if addr >= (end_addr as u8) {
                    return Ok(index);
                } else {
                    addr += 1;
                }
            }
        }

        SerialCommand::Raw { sdi12_cmd } if sdi12_cmd.len() > 0 => {
            let bytes_read = sdi12.query_device(sdi12_cmd.as_bytes(), output).await?;

            bytes_read
        }

        SerialCommand::Raw { sdi12_cmd: _ } => {
            let msg = b"RAW <SDI12_CMD>\r\n";
            output[..msg.len()].copy_from_slice(msg);
            msg.len()
        }

        SerialCommand::Tc => {
            let (temperature, fault) = thermocouples
                .read_temperature()
                .await
                .map_err(|_| Sdi12Error::SpiError)?;
            let mut writer = Buffer::new(output);
            if fault == 0 {
                let negative = temperature < 0;
                let magnitude = i64::from(temperature).abs();
                let whole = magnitude / 128;
                let fraction = (magnitude % 128) * 10_000 / 128;
                write!(
                    writer,
                    "{}{whole}.{fraction:04}\r\n",
                    if negative { "-" } else { "" }
                )
                .map_err(|_| Sdi12Error::InvalidResponse)?;
            } else {
                write!(writer, "TC Fault 0x{fault:02X}\r\n")
                    .map_err(|_| Sdi12Error::InvalidResponse)?;
            }
            writer.len
        }

        SerialCommand::Adc => {
            let id = adc
                .read_register(0x00)
                .await
                .map_err(|_| Sdi12Error::SpiError)?;
            info!("Received id: {=u16:#06x}", id);
            let mut writer = Buffer::new(output);
            write!(writer, "0x{id:04X}\r\n").map_err(|_| Sdi12Error::InvalidResponse)?;
            writer.len
        }

        // TODO: implement a more useful help
        SerialCommand::Help => {
            let msg = b"COMMANDS: PING, SCAN <START>,<END>, RAW <SDI_CMD>, TC <0|1>\r\n";
            output[..msg.len()].copy_from_slice(msg);
            msg.len()
        }
    };

    Ok(bytes_to_send)
}

pub fn parse_cmd<'a>(cmd: &'a str) -> Result<SerialCommand<'a>, Sdi12Error> {
    let cmd = cmd.trim();

    if cmd.is_empty() {
        return Err(Sdi12Error::InvalidSerialCommand);
    }

    let (instruction, args) = match cmd.split_once(' ') {
        Some((inst, rest)) => (inst, rest.trim()),
        None => (cmd, ""),
    };

    match instruction {
        "PING" => Ok(SerialCommand::Ping),

        "SCAN" => {
            let (start, end) = match args.split_once(',') {
                Some((s, e)) => (s.trim(), e.trim()),
                None => ("", ""),
            };

            let start_addr: char = start.chars().next().unwrap_or(' ');
            let end_addr: char = end.chars().next().unwrap_or(' ');
            Ok(SerialCommand::Scan {
                start_addr,
                end_addr,
            })
        }

        "RAW" => Ok(SerialCommand::Raw { sdi12_cmd: args }),

        "TC" => Ok(SerialCommand::Tc),

        "ADC" => Ok(SerialCommand::Adc),

        "HELP" => Ok(SerialCommand::Help),

        _ => Err(Sdi12Error::InvalidSerialCommand),
    }
}

// TODO: more useful error returns
async fn handle_error(tx: &mut UartTx<'_, Async>, error: Sdi12Error) -> Result<(), Error> {
    match error {
        Sdi12Error::Timeout => tx.write(b"SDI12 Timeout\r\n").await,
        Sdi12Error::InvalidSDI12Command => tx.write(b"Invalid SDI12 Command\r\n").await,
        Sdi12Error::InvalidSerialCommand => tx.write(b"Invalid Serial Command\r\n").await,
        Sdi12Error::InvalidResponse => tx.write(b"Invalid SDI12 Response\r\n").await,
        Sdi12Error::SpiError => tx.write(b"TC SPI Error\r\n").await,
        Sdi12Error::UartError => tx.write(b"Uart Error\r\n").await,
    }
}

struct Buffer<'a> {
    bytes: &'a mut [u8],
    len: usize,
}

impl<'a> Buffer<'a> {
    fn new(bytes: &'a mut [u8]) -> Self {
        Self { bytes, len: 0 }
    }
}

impl Write for Buffer<'_> {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        let end = self.len + value.len();
        let destination = self.bytes.get_mut(self.len..end).ok_or(fmt::Error)?;
        destination.copy_from_slice(value.as_bytes());
        self.len = end;
        Ok(())
    }
}
