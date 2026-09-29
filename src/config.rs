use embassy_stm32::Peripherals;
use embassy_stm32::exti::ExtiInput;
use embassy_stm32::gpio::{Input, Level, Output, Pull, Speed};
use embassy_stm32::interrupt::typelevel::EXTI4_15;
use embassy_stm32::mode::Async;
use embassy_stm32::peripherals::IWDG;
use embassy_stm32::spi::MODE_1;
use embassy_stm32::spi::{self, Spi, mode::Master};
use embassy_stm32::usart::{self, BufferedUart};
use embassy_stm32::wdg::IndependentWatchdog;
use embassy_stm32::{bind_interrupts, dma, exti, peripherals};
use static_cell::StaticCell;

use crate::sdi12::Sdi12Bitbang;

bind_interrupts!(pub struct Irqs {
    USART2 => usart::BufferedInterruptHandler<peripherals::USART2>;
    DMA1_CHANNEL2_3 => dma::InterruptHandler<peripherals::DMA1_CH2>, dma::InterruptHandler<peripherals::DMA1_CH3>;
    DMA1_CH4_5_DMAMUX1_OVR => dma::InterruptHandler<peripherals::DMA1_CH4>, dma::InterruptHandler<peripherals::DMA1_CH5>;
    EXTI4_15 => exti::InterruptHandler<EXTI4_15>;
});

static TX_BUF: StaticCell<[u8; 128]> = StaticCell::new();
static RX_BUF: StaticCell<[u8; 128]> = StaticCell::new();

#[allow(dead_code)]
pub struct Link {
    pub spi1: Spi<'static, Async, Master>,
    pub adc_drdy: Input<'static>,
    pub adc_nrst: Output<'static>,

    pub spi2: Spi<'static, Async, Master>,
    pub w5500_nss: Output<'static>,
    pub w5500_intn: ExtiInput<'static, Async>,
    pub w5500_nrst: Output<'static>,
    pub mem_nss: Output<'static>,

    pub serial: BufferedUart<'static>,
    pub sdi12: Sdi12Bitbang<'static>,
    pub status_led: Output<'static>,

    pub watchdog: IndependentWatchdog<'static, IWDG>,
}

impl Link {
    pub fn new(p: Peripherals) -> Self {
        // ADC
        let mut spi1_config = spi::Config::default();
        spi1_config.mode = MODE_1;
        let spi1 = Spi::new(
            p.SPI1,
            p.PB3,
            p.PA12,
            p.PA11,
            p.DMA1_CH2,
            p.DMA1_CH3,
            Irqs,
            spi1_config,
        );
        let adc_drdy = Input::new(p.PC6, Pull::Up);
        let adc_nrst = Output::new(p.PA10, Level::High, Speed::High);

        // W5500 init
        let mut spi2_config = spi::Config::default();
        spi2_config.frequency = embassy_stm32::time::mhz(10);
        let spi2 = Spi::new(
            p.SPI2,
            p.PB8,
            p.PB7,
            p.PB6,
            p.DMA1_CH4,
            p.DMA1_CH5,
            Irqs,
            spi2_config,
        );
        let w5500_nss = Output::new(p.PB5, Level::High, Speed::High);
        let w5500_intn = ExtiInput::new(p.PB4, p.EXTI4, Pull::Up, Irqs);
        let w5500_nrst = Output::new(p.PB9, Level::High, Speed::High);
        let mem_nss = Output::new(p.PA0, Level::High, Speed::High);

        let tx_buf_ref = TX_BUF.init([0; 128]);
        let rx_buf_ref = RX_BUF.init([0; 128]);

        let serial = BufferedUart::new(
            p.USART2,
            p.PA3,
            p.PA2,
            tx_buf_ref,
            rx_buf_ref,
            Irqs,
            usart::Config::default(),
        )
        .unwrap();

        // TODO: update when on Link instead of NUCLEO
        let sdi12 = Sdi12Bitbang::new(p.PA1.into());
        let status_led = Output::new(p.PA4, Level::High, Speed::Low);

        let watchdog = IndependentWatchdog::new(p.IWDG, 15_000_000);

        Link {
            spi1,
            adc_drdy,
            adc_nrst,
            spi2,
            w5500_nss,
            w5500_intn,
            w5500_nrst,
            mem_nss,
            serial,
            sdi12,
            status_led,
            watchdog,
        }
    }
}
