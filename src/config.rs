use crate::sdi12::Sdi12Uart;
use embassy_embedded_hal::shared_bus::asynch::spi::SpiDevice;
use embassy_stm32::Peripherals;
use embassy_stm32::exti::ExtiInput;
use embassy_stm32::gpio::{Input, Level, Output, Pull, Speed};
use embassy_stm32::interrupt::typelevel::EXTI4_15;
use embassy_stm32::mode::Async;
use embassy_stm32::peripherals::IWDG;
use embassy_stm32::spi::{self, MODE_1, Spi, mode::Master};
use embassy_stm32::usart::{self, Uart};
use embassy_stm32::wdg::IndependentWatchdog;
use embassy_stm32::{bind_interrupts, dma, exti, peripherals};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::mutex::Mutex;
use static_cell::StaticCell;

use crate::Ads131m;
use crate::Max31856;

bind_interrupts!(pub struct Irqs {
    USART2_LPUART2 => usart::InterruptHandler<peripherals::USART2>;
    USART3_4_5_6_LPUART1 => usart::InterruptHandler<peripherals::USART3>;
    DMA1_CHANNEL1 => dma::InterruptHandler<peripherals::DMA1_CH1>;
    DMA1_CHANNEL2_3 => dma::InterruptHandler<peripherals::DMA1_CH2>, dma::InterruptHandler<peripherals::DMA1_CH3>;
    DMA1_CH4_7_DMA2_CH1_5_DMAMUX1_OVR  => dma::InterruptHandler<peripherals::DMA1_CH4>,
                                          dma::InterruptHandler<peripherals::DMA1_CH5>,
                                          dma::InterruptHandler<peripherals::DMA1_CH6>,
                                          dma::InterruptHandler<peripherals::DMA1_CH7>,
                                          dma::InterruptHandler<peripherals::DMA2_CH1>,
                                          dma::InterruptHandler<peripherals::DMA2_CH2>;
    EXTI4_15 => exti::InterruptHandler<EXTI4_15>;
});

type SpiBus = Mutex<CriticalSectionRawMutex, Spi<'static, Async, Master>>;

#[allow(dead_code)]
pub struct Link {
    pub adc: Ads131m<'static>,
    pub tc: Max31856<'static>,

    pub eth_device:
        SpiDevice<'static, CriticalSectionRawMutex, Spi<'static, Async, Master>, Output<'static>>,
    pub mem_device:
        SpiDevice<'static, CriticalSectionRawMutex, Spi<'static, Async, Master>, Output<'static>>,
    pub w5500_intn: ExtiInput<'static, Async>,
    pub w5500_nrst: Output<'static>,

    pub serial: Uart<'static, Async>,
    pub sdi12: Sdi12Uart<'static>,
    pub status_led: Output<'static>,

    pub watchdog: IndependentWatchdog<'static, IWDG>,
}

impl Link {
    pub async fn new(p: Peripherals) -> Self {
        // ADC
        let mut spi1_config = spi::Config::default();
        spi1_config.mode = MODE_1;
        let adc_spi = Spi::new(
            p.SPI1,
            p.PB3,
            p.PA12,
            p.PA11,
            p.DMA1_CH2,
            p.DMA1_CH3,
            Irqs,
            spi1_config,
        );

        let adc_nss = Output::new(p.PA15, Level::High, Speed::High);
        let tc_nss = Output::new(p.PA9, Level::High, Speed::High);
        let adc_drdy = Input::new(p.PA6, Pull::Up);
        let adc_nrst = Output::new(p.PA10, Level::High, Speed::High);

        let spi1_bus = SPI_BUS.init(Mutex::new(adc_spi));
        let adc_device = SpiDevice::new(spi1_bus, adc_nss);
        let tc_device = SpiDevice::new(spi1_bus, tc_nss);

        let tc = match Max31856::new(tc_device).await {
            Ok(result) => result,
            _ => defmt::panic!("Max31856 initalization"),
        };

        let adc = Ads131m::new(adc_device, adc_nrst, adc_drdy, 4);

        // W5500 init
        let mut eth_spi_config = spi::Config::default();
        eth_spi_config.frequency = embassy_stm32::time::mhz(10);
        let eth_spi = Spi::new(
            p.SPI2,
            p.PB8,
            p.PB7,
            p.PB6,
            p.DMA1_CH4,
            p.DMA1_CH5,
            Irqs,
            eth_spi_config,
        );

        let w5500_nss = Output::new(p.PB5, Level::High, Speed::High);
        let w5500_intn = ExtiInput::new(p.PB4, p.EXTI4, Pull::Up, Irqs);
        let w5500_nrst = Output::new(p.PB9, Level::High, Speed::High);
        let mem_nss = Output::new(p.PA0, Level::High, Speed::High);

        static SPI_BUS: StaticCell<SpiBus> = StaticCell::new();
        let spi2_bus = SPI_BUS.init(Mutex::new(eth_spi));

        let eth_device = SpiDevice::new(spi2_bus, w5500_nss);
        let mem_device = SpiDevice::new(spi2_bus, mem_nss);

        let serial = defmt::unwrap!(Uart::new(
            p.USART2,
            p.PA3,
            p.PA2,
            p.DMA1_CH6,
            p.DMA1_CH7,
            Irqs,
            usart::Config::default(),
        ));

        let sdi12 = Sdi12Uart::new(p.PB2, p.USART3, p.DMA2_CH1, p.DMA2_CH2, Irqs);
        let status_led = Output::new(p.PA4, Level::High, Speed::Low);

        let watchdog = IndependentWatchdog::new(p.IWDG, 15_000_000);

        Link {
            adc,
            tc,
            eth_device,
            mem_device,
            w5500_intn,
            w5500_nrst,
            serial,
            sdi12,
            status_led,
            watchdog,
        }
    }
}
