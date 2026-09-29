#![no_std]
#![no_main]

use defmt::*;
use defmt_rtt as _;
use embassy_executor::Spawner;
use embassy_stm32::wdg::IndependentWatchdog;
use embassy_stm32::{bind_interrupts, dma, exti, peripherals, usart};
use embassy_time::Timer;

use embassy_net::{Runner as NetRunner, StackResources};
use embassy_net_wiznet::{Device, Runner as WiznetRunner, State as WiznetState};
use embedded_hal_bus::spi::ExclusiveDevice;
use static_cell::StaticCell;

use panic_probe as _;

use crate::config::Link;

mod config;
mod max31856;
mod sdi12;
mod serial;

static WIZNET_STATE: StaticCell<WiznetState<1, 1>> = StaticCell::new(); // Might need to increase size of packet queues so a new packet can be received while processing an old one, but watch out for RAM usage
static NET_RESOURCES: StaticCell<StackResources<2>> = StaticCell::new();

type W5500SpiBus =
    embassy_stm32::spi::Spi<'static, embassy_stm32::mode::Async, embassy_stm32::spi::mode::Master>;
type W5500CsPin = embassy_stm32::gpio::Output<'static>;
type W5500IntPin = embassy_stm32::exti::ExtiInput<'static, embassy_stm32::mode::Async>;
type W5500RstPin = embassy_stm32::gpio::Output<'static>;
type W5500SpiDevice = ExclusiveDevice<W5500SpiBus, W5500CsPin, embassy_time::Delay>;
type W5500Runner = WiznetRunner<
    'static,
    embassy_net_wiznet::chip::W5500,
    W5500SpiDevice,
    W5500IntPin,
    W5500RstPin,
>;

// interrupts for serial bus
bind_interrupts!(pub struct Irqs {
    USART1 => usart::InterruptHandler<peripherals::USART1>;
    USART2 => usart::InterruptHandler<peripherals::USART2>;
    DMA1_CHANNEL2_3 => dma::InterruptHandler<peripherals::DMA1_CH2>, dma::InterruptHandler<peripherals::DMA1_CH3>;
    DMA1_CH4_5_DMAMUX1_OVR => dma::InterruptHandler<peripherals::DMA1_CH4>, dma::InterruptHandler<peripherals::DMA1_CH5>;
    EXTI4_15 => exti::InterruptHandler<embassy_stm32::interrupt::typelevel::EXTI4_15>;
});

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_stm32::init(Default::default());
    let mut link = Link::new(p);

    link.watchdog.unleash();
    spawner.spawn(feed_watchdog(link.watchdog).unwrap());

    // Combine SPI and CS into a SpiDevice
    let spi_device = ExclusiveDevice::new(link.spi2, link.w5500_nss, embassy_time::Delay).unwrap();

    // Initialize the Wiznet driver
    let mac_addr = [0x02, 0x00, 0x00, 0x00, 0x00, 0x01];
    let (w5500_device, w5500_runner) = embassy_net_wiznet::new(
        mac_addr,
        WIZNET_STATE.init(WiznetState::new()),
        spi_device,
        link.w5500_intn,
        link.w5500_nrst,
    )
    .await
    .unwrap();

    // Initialize the Embassy network stack
    let net_config = embassy_net::Config::dhcpv4(Default::default());
    let seed = 1234;
    let resources = NET_RESOURCES.init(StackResources::new());
    let (stack, net_runner) = embassy_net::new(w5500_device, net_config, resources, seed);
    let _ = stack;

    spawner.spawn(w5500_task(w5500_runner).unwrap());
    spawner.spawn(net_task(net_runner).unwrap());

    Timer::after_millis(250).await;

    info!("Starting Program...");

    // TODO: what happens if usart errors out?
    loop {
        info!("Reading!");
        match serial::receive(&mut link.serial, &mut link.sdi12).await {
            Ok(_) => {
                info!("Received Command",);
            }
            Err(e) => {
                warn!("Error, {:?}", e);
            }
        }
    }
}

#[embassy_executor::task]
async fn feed_watchdog(mut watchdog: IndependentWatchdog<'static, peripherals::IWDG>) -> ! {
    loop {
        watchdog.pet();
        Timer::after_secs(5).await;
    }
}

// Background Task for the W5500 hardware driver
#[embassy_executor::task]
async fn w5500_task(runner: W5500Runner) -> ! {
    runner.run().await
}

// Background Task for the Embassy TCP/IP stack
#[embassy_executor::task]
async fn net_task(mut runner: NetRunner<'static, Device<'static>>) -> ! {
    runner.run().await
}
