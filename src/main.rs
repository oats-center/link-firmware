#![no_std]
#![no_main]

// Remove this after testing
#![allow(dead_code)]

use defmt::*;
use defmt_rtt as _;
use embassy_executor::Spawner;
use embassy_stm32::wdg::IndependentWatchdog;
use embassy_stm32::peripherals;
use embassy_time::Timer;
use panic_probe as _;

mod config;
mod ads131m;
mod max31856;
mod sdi12;
mod serial;
mod ethernet;
mod protocol;
mod tcp_server;

use crate::config::Link;

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_stm32::init(Default::default());
    let mut link = Link::new(p).await;

    // TODO: Feed watchdog only when critical tasks are making progress
    link.watchdog.unleash();
    spawner.spawn(feed_watchdog(link.watchdog).unwrap());

    info!("Starting Program...");

    ethernet::start(
        spawner,
        link.eth_device,
        link.w5500_intn,
        link.w5500_nrst,
    )
    .await;

    // link.adc.reset().await;

    loop {
        // let _ = match link.adc.read_channels().await {
        //     Ok(r) => { 
        //         info!("Ch1: {=i32:#08x} | Ch2: {=i32:#08x} | Ch3: {=i32:#08x} | Ch4: {=i32:#08x} ", r[0], r[1], r[2], r[3]); 
        //     },
        //     Err(e) => { info!("Error: {:?}", e); },
        // };

        Timer::after_millis(500).await;
    }
}

#[embassy_executor::task]
async fn feed_watchdog(mut watchdog: IndependentWatchdog<'static, peripherals::IWDG>) -> ! {
    loop {
        watchdog.pet();
        Timer::after_secs(5).await;
    }
}