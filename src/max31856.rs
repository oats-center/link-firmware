use embassy_stm32::gpio::{Level, Output, Speed};
use embassy_stm32::mode::Blocking;
use embassy_stm32::spi::{self, Spi, mode::Master};

#[allow(dead_code)]
pub struct Max31856<'a> {
    spi: Spi<'a, Blocking, Master>,
    cs: [Output<'a>; 2],
}

#[allow(dead_code)]
impl<'a> Max31856<'a> {
    pub fn new(
        spi: Spi<'a, Blocking, Master>,
        cs0: embassy_hal_internal::Peri<'a, embassy_stm32::peripherals::PA4>,
        cs1: embassy_hal_internal::Peri<'a, embassy_stm32::peripherals::PA5>,
    ) -> Result<Self, spi::Error> {
        let mut device = Self {
            spi,
            cs: [
                Output::new(cs0, Level::High, Speed::Low),
                Output::new(cs1, Level::High, Speed::Low),
            ],
        };

        for channel in 0..2 {
            device.write_register(channel, 0x00, 0x90)?; // continuous conversion, open-circuit detection, 60 Hz
            device.write_register(channel, 0x01, 0x03)?; // K-type thermocouple
        }

        Ok(device)
    }

    pub fn read_temperature(&mut self, channel: usize) -> Result<(i32, u8), spi::Error> {
        let mut data = [0x0c, 0, 0, 0, 0];
        self.cs[channel].set_low();
        let result = self.spi.blocking_transfer_in_place(&mut data);
        self.cs[channel].set_high();
        result?;

        let value = i32::from_be_bytes([
            if data[1] & 0x80 != 0 { 0xff } else { 0 },
            data[1],
            data[2],
            data[3],
        ]) >> 5;
        Ok((value, data[4]))
    }

    fn write_register(
        &mut self,
        channel: usize,
        register: u8,
        value: u8,
    ) -> Result<(), spi::Error> {
        self.cs[channel].set_low();
        let result = self.spi.blocking_write(&[register | 0x80, value]);
        self.cs[channel].set_high();
        result
    }
}
