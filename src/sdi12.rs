use defmt::{debug, warn};
use embassy_hal_internal::Peri;
use embassy_stm32::gpio::{AnyPin, Flex, Level, Output, Pull, Speed};
use embassy_stm32::mode::Async;
use embassy_stm32::peripherals::{DMA1_CH4, DMA1_CH5, PA9, USART1};
use embassy_stm32::usart::{self, Config, Uart};
use embassy_time::{Duration, Instant, Timer, with_timeout};

use crate::config::Irqs;

#[derive(defmt::Format, PartialEq)]
pub enum SerialCommand<'a> {
    Ping,
    Scan { start_addr: char, end_addr: char },
    Raw { sdi12_cmd: &'a str },
    Help,
}

#[derive(defmt::Format, PartialEq)]
#[allow(dead_code)]
pub enum Sdi12Error {
    Timeout,
    InvalidSDI12Command,
    InvalidSerialCommand,
    InvalidResponse,
    UartError,
}

pub trait Sdi12Bus {
    async fn query_device(&mut self, cmd: &[u8], rx_buf: &mut [u8]) -> Result<usize, Sdi12Error>;
}

pub struct Sdi12Bitbang<'a> {
    pin: Flex<'a>,
}

impl<'a> Sdi12Bitbang<'a> {
    const BIT_US: u64 = 833;

    pub fn new(pin: Peri<'a, AnyPin>) -> Self {
        let mut pin = Flex::new(pin);
        pin.set_low();
        pin.set_as_input(Pull::Down);
        Self { pin }
    }

    fn write_byte(&mut self, byte: u8) {
        let parity = (byte & 0x7f).count_ones() & 1 != 0;
        let bits = [
            false,
            byte & 0x01 != 0,
            byte & 0x02 != 0,
            byte & 0x04 != 0,
            byte & 0x08 != 0,
            byte & 0x10 != 0,
            byte & 0x20 != 0,
            byte & 0x40 != 0,
            parity,
            true,
        ];
        let start = Instant::now();

        for (index, bit) in bits.into_iter().enumerate() {
            self.set_logical_level(bit);
            Self::wait_until(start + Duration::from_micros((index as u64 + 1) * Self::BIT_US));
        }
    }

    fn wait_for_start(&self, timeout: Duration) -> Result<(), Sdi12Error> {
        let deadline = Instant::now() + timeout;
        while self.pin.is_high() {
            if Instant::now() >= deadline {
                warn!("SDI-12 bus stayed SPACE/high; never reached MARK/low");
                return Err(Sdi12Error::Timeout);
            }
        }
        while !self.pin.is_high() {
            if Instant::now() >= deadline {
                warn!("SDI-12 bus stayed MARK/low; no response start bit");
                return Err(Sdi12Error::Timeout);
            }
        }
        Ok(())
    }

    fn set_logical_level(&mut self, high: bool) {
        if high {
            self.pin.set_low();
        } else {
            self.pin.set_high();
        }
    }

    fn logical_level(&self) -> bool {
        self.pin.is_low()
    }

    fn wait_until(deadline: Instant) {
        while Instant::now() < deadline {}
    }

    fn receive_response(&self, rx_buf: &mut [u8]) -> Result<usize, Sdi12Error> {
        let mut index = 0;
        let mut timeout = Duration::from_millis(100);

        loop {
            if self.wait_for_start(timeout).is_err() {
                if index == 0 {
                    warn!("SDI-12 response timeout waiting for first start bit");
                } else {
                    warn!(
                        "SDI-12 inter-character timeout: received={=usize} partial={=[u8]}",
                        index,
                        &rx_buf[..index]
                    );
                }
                return Err(Sdi12Error::Timeout);
            }

            let start = Instant::now();
            let mut byte = 0u8;
            for bit in 0..7 {
                Self::wait_until(start + Duration::from_micros(Self::BIT_US * (3 + 2 * bit) / 2));
                if self.logical_level() {
                    byte |= 1 << bit;
                }
            }

            Self::wait_until(start + Duration::from_micros(Self::BIT_US * 17 / 2));
            let parity = self.logical_level();
            Self::wait_until(start + Duration::from_micros(Self::BIT_US * 19 / 2));
            let expected_parity = byte.count_ones() & 1 != 0;
            if parity != expected_parity {
                warn!(
                    "SDI-12 parity error: byte={=u8:#04x} received={=bool} expected={=bool}",
                    byte, parity, expected_parity
                );
                return Err(Sdi12Error::InvalidResponse);
            }
            if !self.logical_level() {
                warn!("SDI-12 stop-bit error: byte={=u8:#04x}", byte);
                return Err(Sdi12Error::InvalidResponse);
            }

            if index >= rx_buf.len() {
                warn!(
                    "SDI-12 response buffer full: capacity={=usize}",
                    rx_buf.len()
                );
                return Err(Sdi12Error::InvalidResponse);
            }
            rx_buf[index] = byte;
            index += 1;

            if byte == b'\n' {
                return Ok(index);
            }
            timeout = Duration::from_millis(15);
        }
    }
}

impl<'a> Sdi12Bus for Sdi12Bitbang<'a> {
    async fn query_device(&mut self, cmd: &[u8], rx_buf: &mut [u8]) -> Result<usize, Sdi12Error> {
        debug!("SDI-12 transaction start: command={=[u8]}", cmd);

        self.pin.set_high();
        self.pin.set_as_output(Speed::Low);
        Self::wait_until(Instant::now() + Duration::from_millis(12));

        self.pin.set_low();
        Self::wait_until(Instant::now() + Duration::from_micros(8_330));

        for &byte in cmd {
            self.write_byte(byte);
        }

        self.pin.set_low();
        self.pin.set_as_input(Pull::Down);
        let result = self.receive_response(rx_buf);
        match result {
            Ok(size) => debug!(
                "SDI-12 response complete: response={=[u8]}",
                &rx_buf[..size]
            ),
            Err(ref error) => warn!("SDI-12 transaction failed: error={:?}", error),
        }
        result
    }
}

/*
pub struct Sdi12Uart<'a> {
    uart: Peri<'a, USART1>,
    pin: Peri<'a, PA9>,
    tx_dma: Peri<'a, DMA1_CH4>,
    rx_dma: Peri<'a, DMA1_CH5>,
    irqs: Irqs,
}

impl<'a> Sdi12Uart<'a> {
    pub fn new(
        pin: Peri<'a, PA9>,
        uart: Peri<'a, USART1>,
        tx_dma: Peri<'a, DMA1_CH4>,
        rx_dma: Peri<'a, DMA1_CH5>,
        irqs: Irqs,
    ) -> Self {
        Sdi12Uart {
            pin,
            uart,
            tx_dma,
            rx_dma,
            irqs,
        }
    }

    async fn receive_response(
        uart: &mut Uart<'_, Async>,
        rx_buf: &mut [u8],
    ) -> Result<usize, Sdi12Error> {
        let mut character = [0u8; 1];
        let mut index: usize = 0;

        let mut timeout = embassy_time::Duration::from_millis(100);

        loop {
            if index >= rx_buf.len() {
                return Err(Sdi12Error::InvalidResponse); // response too long
            }

            match with_timeout(timeout, uart.read(&mut character))
                .await
                .map_err(|_| Sdi12Error::Timeout)?
            {
                Ok(()) => {
                    let no_parity = character[0] & 0x7F;

                    rx_buf[index] = no_parity;
                    index += 1;

                    if no_parity == '\n' as u8 {
                        return Ok(index);
                    }

                    timeout = embassy_time::Duration::from_millis(15);
                }
                Err(_e) => {
                    return Err(Sdi12Error::UartError);
                }
            };
        }
    }
}

impl<'a> Sdi12Bus for Sdi12Uart<'a> {
    async fn query_device(&mut self, cmd: &[u8], rx_buf: &mut [u8]) -> Result<usize, Sdi12Error> {
        {
            let mut break_pin = Output::new(self.pin.reborrow(), Level::High, Speed::Low);

            Timer::after_micros(12000).await;

            break_pin.set_low();
            Timer::after_micros(8330).await;
        }

        // TODO: should config be a const?
        let mut config = Config::default();

        config.baudrate = 1200;
        config.data_bits = usart::DataBits::DataBits7;
        config.parity = usart::Parity::ParityEven;
        config.invert_rx = true;
        config.invert_tx = true;

        let mut uart_hd = Uart::new_half_duplex(
            self.uart.reborrow(),
            self.pin.reborrow(),
            self.tx_dma.reborrow(),
            self.rx_dma.reborrow(),
            self.irqs,
            config,
            usart::HalfDuplexReadback::NoReadback,
        )
        .map_err(|_| Sdi12Error::UartError)?;

        uart_hd
            .write(cmd)
            .await
            .map_err(|_| Sdi12Error::UartError)?;

        Self::receive_response(&mut uart_hd, rx_buf).await
    }
}

*/