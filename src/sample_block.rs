// The eventual ADC reader and the synthetic source both produce this type

pub const SAMPLE_RATE_HZ: u32 = 2_000; // demo default, not runtime-configurable yet
pub const CHANNEL_COUNT: usize = 2;
pub const SCANS_PER_BLOCK: usize = 200;
pub const SAMPLE_BYTES: usize = core::mem::size_of::<i32>();
pub const RAW_SAMPLE_BYTES: usize = CHANNEL_COUNT * SCANS_PER_BLOCK * SAMPLE_BYTES;

#[derive(Clone, Copy)]
pub struct SampleBlock {
    pub first_scan_index: u64,
    pub scan_count: u16,
    pub sample_rate_hz: u32,
    pub samples: [[i32; CHANNEL_COUNT]; SCANS_PER_BLOCK],
}

/// Channel 0 is a 10-Hz sawtooth; channel 1 is a 5-Hz square wave.
/// The count values are deterministic and fit into signed 24-bit samples.
/// Caller provides a nonzero sample rate divisible by 10.
pub fn synthetic_scan(scan: u64, sample_rate_hz: u32) -> [i32; CHANNEL_COUNT] {
    let period = (sample_rate_hz / 10) as u64;
    let phase = scan % period;
    let ramp = ((phase * 200 / period) as i32 - 100) * 1_000;
    let square = if ((scan / period) & 1) == 0 { 250_000 } else { -250_000 };
    [ramp, square]
}

pub fn synthetic_block(first_scan_index: u64, scan_count: u16, sample_rate_hz: u32) -> SampleBlock {
    assert!(scan_count > 0 && (scan_count as usize) <= SCANS_PER_BLOCK);
    assert!(sample_rate_hz >= 20 && sample_rate_hz % 10 == 0);

    let mut block = SampleBlock {
        first_scan_index,
        scan_count,
        sample_rate_hz,
        samples: [[0; CHANNEL_COUNT]; SCANS_PER_BLOCK],
    };
    for i in 0..scan_count as usize {
        block.samples[i] = synthetic_scan(first_scan_index + i as u64, sample_rate_hz);
    }
    block
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waveforms_and_indices_are_stable() {
        assert_eq!(synthetic_scan(0, 2_000), [-100_000, 250_000]);
        assert_eq!(synthetic_scan(100, 2_000), [0, 250_000]);
        assert_eq!(synthetic_scan(200, 2_000), [-100_000, -250_000]);
        assert_eq!(synthetic_scan(400, 2_000), [-100_000, 250_000]);
        let block = synthetic_block(150, 100, 2_000);
        assert_eq!(block.first_scan_index, 150);
        assert_eq!(block.scan_count, 100);
        assert_eq!(block.samples[0], synthetic_scan(150, 2_000));
        assert_eq!(block.samples[99], synthetic_scan(249, 2_000));
        assert_eq!(block.samples[100], [0, 0]);
    }

    #[test]
    fn full_size_and_full_rate() {
        let block = synthetic_block(0, 200, 64_000);
        assert_eq!(block.scan_count, 200);
        assert_eq!(block.sample_rate_hz, 64_000);
        assert_eq!(block.samples[199], synthetic_scan(199, 64_000));
    }
}

