// Creates 100 deterministic two-channel scans every 50 ms (20 blocks/sec)
// Does NOT emulate the real ADC's 500-us DRDY/SPI timing or clock jitter

use defmt::info;
use embassy_time::{Duration, Ticker, Timer};
use crate::{acquisition, sample_block::{synthetic_block, SCANS_PER_BLOCK}};

#[embassy_executor::task]
pub async fn simulator_task() -> ! {
    loop {
        // Wait until the host requests START. Avoid emitting stale data when idle
        while !acquisition::is_streaming() {
            Timer::after_millis(5).await;
        }

        info!("Simulator: session started at 2000 scans/sec");
        let session_id = acquisition::session_id();
        let mut first_scan: u64 = 0;
        let mut ticker = Ticker::every(Duration::from_millis(50));
        ticker.reset_after(Duration::from_millis(50));

        while acquisition::is_streaming() && acquisition::session_id() == session_id {
            ticker.next().await;
            if !acquisition::is_streaming() || acquisition::session_id() != session_id {
                break;
            }

            let block = synthetic_block(first_scan);
            // Advance regardless of queue overflow. Gaps are observable on host
            first_scan = first_scan.wrapping_add(SCANS_PER_BLOCK as u64);
            if acquisition::SAMPLE_QUEUE.try_send(block).is_err() {
                acquisition::report_drop();
            }
        }
        info!("Simulator: session stopped");
    }
}
