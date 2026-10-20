use defmt::info;
use embassy_embedded_hal::shared_bus::asynch::spi::SpiDevice;
use embassy_executor::Spawner;
use embassy_net::{
    Config, Ipv4Address, Ipv4Cidr, Runner as NetRunner,
    StackResources, StaticConfigV4,
};
use embassy_net_wiznet::{
    chip::W5500, Device as WiznetDevice, Runner as WiznetRunner, State as WiznetState,
};
use embassy_stm32::{
    exti::ExtiInput,
    gpio::Output,
    mode::Async,
    spi::{mode::Master, Spi},
};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use static_cell::StaticCell;

pub const TCP_PORT: u16 = 7600;
const BOARD_IP: Ipv4Address = Ipv4Address::new(192, 168, 1, 120);
const NETWORK_PREFIX_BITS: u8 = 24;

// Each board will need its own MAC_ADDR
const MAC_ADDR: [u8; 6] = [0x02, 0x41, 0x56, 0x44, 0x51, 0x01];

// Test seed only. Need to replace with an unpredictable per-boot seed before deployment
const TCP_RANDOM_SEED: u64 = 0x41_56_44_51_5A_77_00_01;

pub type EthernetSpiDevice = SpiDevice<
    'static,
    CriticalSectionRawMutex,
    Spi<'static, Async, Master>,
    Output<'static>,
>;

type WiznetInt = ExtiInput<'static, Async>;
type WiznetRst = Output<'static>;

type HwRunner = WiznetRunner<'static, W5500, EthernetSpiDevice, WiznetInt, WiznetRst>;
type IpRunner = NetRunner<'static, WiznetDevice<'static>>;

static WIZNET_STATE: StaticCell<WiznetState<2, 2>> = StaticCell::new();
static NET_RESOURCES: StaticCell<StackResources<2>> = StaticCell::new();

// Initialize W5500 and embassy-net, then spawn all networking tasks
pub async fn start(
    spawner: Spawner,
    spi_device: EthernetSpiDevice,
    int_pin: WiznetInt,
    reset_pin: WiznetRst,
) {
    let state = WIZNET_STATE.init(WiznetState::new());

    // Explicit W5500 type is required to choose a chip implementation
    let (device, hw_runner) = defmt::unwrap!(
        embassy_net_wiznet::new::<2, 2, W5500, _, _, _>(
            MAC_ADDR,
            state,
            spi_device,
            int_pin,
            reset_pin,
        )
        .await
    );

    info!("W5500 reset and SPI initialization succeeded");
    spawner.spawn(w5500_task(hw_runner).unwrap());

    let config = Config::ipv4_static(StaticConfigV4 {
        address: Ipv4Cidr::new(BOARD_IP, NETWORK_PREFIX_BITS),
        gateway: None, // direct connection / same-subnet LAN; no router needed
        dns_servers: Default::default(),
    });

    let (stack, net_runner) = embassy_net::new(
        device,
        config,
        NET_RESOURCES.init(StackResources::new()),
        TCP_RANDOM_SEED,
    );

    spawner.spawn(net_task(net_runner).unwrap());
    spawner.spawn(crate::tcp_server::tcp_server_task(stack).unwrap());

    info!("Static IPv4 configured: 192.168.1.120/24");
}

#[embassy_executor::task]
async fn w5500_task(runner: HwRunner) -> ! {
    runner.run().await
}

#[embassy_executor::task]
async fn net_task(mut runner: IpRunner) -> ! {
    runner.run().await
}