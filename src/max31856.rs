use core::convert::Infallible;

use embassy_embedded_hal::shared_bus::SpiDeviceError;
use embassy_embedded_hal::shared_bus::asynch::spi::SpiDevice;
use embassy_stm32::gpio::Output;
use embassy_stm32::mode::Async;
use embassy_stm32::spi::{Spi, mode::Master};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embedded_hal_async::spi::SpiDevice as _;

#[allow(dead_code)]
pub struct Max31856<'a> {
    spi_device: SpiDevice<'a, CriticalSectionRawMutex, Spi<'a, Async, Master>, Output<'a>>,
}

#[allow(dead_code)]
impl<'a> Max31856<'a> {
    pub async fn new(
        spi_device: SpiDevice<'a, CriticalSectionRawMutex, Spi<'a, Async, Master>, Output<'a>>,
    ) -> Result<Self, SpiDeviceError<embassy_stm32::spi::Error, Infallible>> {
        let mut device = Self { spi_device };

        device.write_register(0x00, 0x90).await?; // continuous conversion, open-circuit detection, 60 Hz
        device.write_register(0x01, 0x03).await?; // K-type thermocouple

        Ok(device)
    }

    pub async fn read_temperature(
        &mut self,
    ) -> Result<(i32, u8), SpiDeviceError<embassy_stm32::spi::Error, Infallible>> {
        let mut data = [0x0c, 0, 0, 0, 0];

        let result = self.spi_device.transfer_in_place(&mut data).await;
        result?;

        let value = i32::from_be_bytes([
            if data[1] & 0x80 != 0 { 0xff } else { 0 },
            data[1],
            data[2],
            data[3],
        ]) >> 5;
        Ok((value, data[4]))
    }

    async fn write_register(
        &mut self,
        register: u8,
        value: u8,
    ) -> Result<(), SpiDeviceError<embassy_stm32::spi::Error, Infallible>> {
        let result = self.spi_device.write(&[register | 0x8, value]).await;
        result
    }
}
