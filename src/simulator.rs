// Creates 100 deterministic two-channel scans every 50 ms (20 blocks/sec)
// Does NOT emulate the real ADC's 500-us DRDY/SPI timing or clock jitter

use defmt::info;
use embassy_time::{Duration, Ticker, Timer};
use crate::{acquisition, sample_block::{synthetic_block, SCANS_PER_BLOCK, SAMPLE_RATE_HZ}};

#[embassy_executor::task]
pub async fn simulator_task() -> ! {
    loop {
        while !acquisition::is_streaming() {
            Timer::after_millis(5).await;
        }
        let session_id = acquisition::session_id();
        let sample_rate_hz = SAMPLE_RATE_HZ;
        let scan_count = SCANS_PER_BLOCK as u16; // max 200, may be smaller later
        let interval_us = (scan_count as u64 * 1_000_000) / sample_rate_hz as u64;
        let interval = Duration::from_micros(interval_us);
        info!("Simulator: rate={=u32} scans/s block={=u16}", sample_rate_hz, scan_count);
        let mut first_scan: u64 = 0;
        let mut ticker = Ticker::every(interval);
        ticker.reset_after(interval);

        while acquisition::is_streaming() && acquisition::session_id() == session_id {
            ticker.next().await;
            if !acquisition::is_streaming() || acquisition::session_id() != session_id {
                break;
            }
            let block = synthetic_block(first_scan, scan_count, sample_rate_hz);
            // Account for scheduled samples even if the network queue is full.
            first_scan = first_scan.wrapping_add(scan_count as u64);
            if acquisition::SAMPLE_QUEUE.try_send(block).is_err() {
                acquisition::report_drop();
            }
        }
        info!("Simulator: session stopped");
    }
}
