use core::convert::Infallible;

use embassy_embedded_hal::shared_bus::SpiDeviceError;
use embassy_embedded_hal::shared_bus::asynch::spi::SpiDevice;
use embassy_stm32::gpio::{Input, Output};
use embassy_stm32::mode::Async;
use embassy_stm32::spi::{Spi, mode::Master};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_time::Timer;
use embedded_hal_async::spi::SpiDevice as _;

const CMD_NULL: u16 = 0x0000;
const CMD_RREG: u16 = 0xA000;
const CMD_WREG: u16 = 0x6000;

const MAX_FRAME_BYTES: usize = 18;

type AdcError = SpiDeviceError<embassy_stm32::spi::Error, Infallible>;

#[allow(dead_code)]
pub struct Ads131m<'a> {
    spi_device: SpiDevice<'a, CriticalSectionRawMutex, Spi<'a, Async, Master>, Output<'a>>,
    nrst: Output<'a>,
    drdy: Input<'a>,
    num_channels: usize,
}

#[allow(dead_code)]
impl<'a> Ads131m<'a> {
    pub fn new(
        spi_device: SpiDevice<'a, CriticalSectionRawMutex, Spi<'a, Async, Master>, Output<'a>>,
        nrst: Output<'a>,
        drdy: Input<'a>,
        num_channels: usize,
    ) -> Self {
        let adc = Self {
            spi_device,
            nrst,
            drdy,
            num_channels,
        };

        adc
    }

    fn frame_len(&self) -> usize {
        (self.num_channels + 2) * 3
    }

    pub async fn reset(&mut self) {
        self.nrst.set_low();
        Timer::after_millis(2).await;
        self.nrst.set_high();
        Timer::after_millis(2).await;
    }

    async fn transfer_frame(
        &mut self,
        command: u16,
        data: Option<u16>,
    ) -> Result<[u8; MAX_FRAME_BYTES], AdcError> {
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

    pub async fn read_register(&mut self, register: u8) -> Result<u16, AdcError> {
        let command = CMD_RREG | (((register & 0x3F) as u16) << 7);

        self.transfer_frame(command, None).await?;

        let response = self.transfer_frame(CMD_NULL, None).await?;

        Ok(u16::from_be_bytes([response[0], response[1]]))
    }

    pub async fn write_register(&mut self, register: u8, value: u16) -> Result<(), AdcError> {
        self.unlock().await?;

        let cmd = CMD_WREG | (((register & 0x3F) as u16) << 7);
        self.transfer_frame(cmd, Some(value)).await?;

        self.lock().await?;
        Ok(())
    }

    async fn lock(&mut self) -> Result<(), AdcError> {
        let cmd: u16 = 0x0555;

        self.transfer_frame(cmd, None).await?;

        Ok(())
    }

    async fn unlock(&mut self) -> Result<(), AdcError> {
        let cmd = 0x0666;

        self.transfer_frame(cmd, None).await?;

        Ok(())
    }

    pub async fn read_channels(&mut self) -> Result<[i32; 4], AdcError> {
        let cmd = 0x0000;

        // while self.drdy.is_high() {
        //     Timer::after_micros(1).await;
        // }

        let r = self.transfer_frame(cmd, None).await?;

        let mut data: [i32; 4] = [0; 4];

        for index in 1..5 {
            data[index - 1] = i32::from_be_bytes([
                if r[index*3] & 0x80 != 0 { 0xff } else { 0 },
                r[index * 3],
                r[index * 3 + 1],
                r[index * 3 + 2],
            ]);
        }

        Ok(data)
    }
}
