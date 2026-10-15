use core::convert::Infallible;

use embassy_embedded_hal::shared_bus::SpiDeviceError;
use embassy_embedded_hal::shared_bus::asynch::spi::SpiDevice;
use embassy_stm32::gpio::{Input, Output};
use embassy_stm32::mode::Async;
use embassy_stm32::spi::{Spi, mode::Master};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embedded_hal_async::spi::SpiDevice as _;

const CMD_NULL: u16 = 0x0000;
const CMD_RREG: u16 = 0xA000;
const CMD_WREG: u16 = 0x6000;

const MAX_FRAME_BYTES: usize = 30;

#[allow(dead_code)]
pub struct Ads131m<'a> {
    spi_device: SpiDevice<'a, CriticalSectionRawMutex, Spi<'a, Async, Master>, Output<'a>>,
    reset: Output<'a>,
    drdy: Input<'a>,
    num_channels: usize,
}

#[allow(dead_code)]
impl<'a> Ads131m<'a> {
    pub fn new(
        spi_device: SpiDevice<'a, CriticalSectionRawMutex, Spi<'a, Async, Master>, Output<'a>>,
        mut reset: Output<'a>,
        drdy: Input<'a>,
        num_channels: usize,
    ) -> Self {
        reset.set_high();

        let adc = Self {
            spi_device,
            reset,
            drdy,
            num_channels,
        };

        adc
    }

    fn frame_len(&self) -> usize {
        (self.num_channels + 2) * 3
    }

    async fn transfer_frame(
        &mut self,
        command: u16,
        data: Option<u16>,
    ) -> Result<[u8; MAX_FRAME_BYTES], SpiDeviceError<embassy_stm32::spi::Error, Infallible>> {
        let mut buffer = [0u8; MAX_FRAME_BYTES];
        let len = self.frame_len();

        buffer[0] = (command >> 8) as u8;
        buffer[1] = command as u8;

        if let Some(value) = data {
            buffer[3] = (value >> 8) as u8;
            buffer[4] = value as u8;
        }

        let result = self.spi_device.transfer_in_place(&mut buffer[..len]).await;
        result?;

        Ok(buffer)
    }

    pub async fn read_register(
        &mut self,
        register: u8,
    ) -> Result<u16, SpiDeviceError<embassy_stm32::spi::Error, Infallible>> {
        let command = CMD_RREG | (((register & 0x3F) as u16) << 7);

        self.transfer_frame(command, None).await?;

        let response = self.transfer_frame(CMD_NULL, None).await?;

        Ok(u16::from_be_bytes([response[0], response[1]]))
    }

    async fn write_register(
        &mut self,
        register: u8,
        value: u16,
    ) -> Result<(), SpiDeviceError<embassy_stm32::spi::Error, Infallible>> {
        let cmd = CMD_WREG | (((register & 0x3F) as u16) << 7);

        self.transfer_frame(cmd, Some(value)).await?;
        Ok(())
    }
}
