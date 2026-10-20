// The eventual ADC reader and the synthetic source both produce this type

pub const SAMPLE_RATE_HZ: u32 = 2_000;
pub const CHANNEL_COUNT: usize = 2;
pub const SCANS_PER_BLOCK: usize = 100;
pub const SAMPLE_BYTES: usize = core::mem::size_of::<i32>();
pub const RAW_SAMPLE_BYTES: usize = CHANNEL_COUNT * SCANS_PER_BLOCK * SAMPLE_BYTES;

#[derive(Clone, Copy)]
pub struct SampleBlock {
    pub first_scan_index: u64,
    // Interleaved by scan: [channel_0, channel_1]. Values are signed ADC counts
    pub samples: [[i32; CHANNEL_COUNT]; SCANS_PER_BLOCK],
}

// Deterministic, bounded, raw ADC-count test signals for protocol testing
// Channel 0: 10 Hz sawtooth, Channel 1: 5 Hz square wave at 2 kHz
// No real ADC is sampled. These values are not measurements or voltages
pub fn synthetic_scan(scan: u64) -> [i32; CHANNEL_COUNT] {
    let ramp = ((scan % 200) as i32 - 100) * 1_000;
    let square = if ((scan / 200) & 1) == 0 {
        250_000
    } else {
        -250_000
    };
    [ramp, square]
}

pub fn synthetic_block(first_scan_index: u64) -> SampleBlock {
    let mut block = SampleBlock {
        first_scan_index,
        samples: [[0; CHANNEL_COUNT]; SCANS_PER_BLOCK],
    };
    for (i, scan) in block.samples.iter_mut().enumerate() {
        *scan = synthetic_scan(first_scan_index + i as u64);
    }
    block
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waveforms_and_indices_are_stable() {
        assert_eq!(synthetic_scan(0), [-100_000, 250_000]);
        assert_eq!(synthetic_scan(100), [0, 250_000]);
        assert_eq!(synthetic_scan(200), [-100_000, -250_000]);
        assert_eq!(synthetic_scan(400), [-100_000, 250_000]);
        let block = synthetic_block(150);
        assert_eq!(block.first_scan_index, 150);
        assert_eq!(block.samples[0], synthetic_scan(150));
        assert_eq!(block.samples[99], synthetic_scan(249));
    }
}
