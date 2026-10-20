// Bounded producer/consumer handoff for real or synthetic SampleBlocks
// `try_send` never blocks a timing-sensitive producer on the Ethernet stack

use embassy_sync::{
    blocking_mutex::raw::CriticalSectionRawMutex,
    channel::Channel,
};
use portable_atomic::{AtomicBool, AtomicU32, Ordering};
use crate::sample_block::SampleBlock;

pub const QUEUE_CAPACITY: usize = 4;

pub static SAMPLE_QUEUE: Channel<CriticalSectionRawMutex, SampleBlock, QUEUE_CAPACITY> =
    Channel::new();

static STREAMING: AtomicBool = AtomicBool::new(false);
static DROPPED_BLOCKS: AtomicU32 = AtomicU32::new(0);
static SESSION_ID: AtomicU32 = AtomicU32::new(0);

pub fn begin_session() {
    // This is a single-client demo. Only tcp_server calls begin/end
    STREAMING.store(false, Ordering::Release);
    SAMPLE_QUEUE.clear();
    DROPPED_BLOCKS.store(0, Ordering::Relaxed);
    // Bump even when STOP and START arrive before the producer wakes up
    // The producer notices the epoch change and restarts at scan index 0
    SESSION_ID.fetch_add(1, Ordering::AcqRel);
    STREAMING.store(true, Ordering::Release);
}

pub fn end_session() {
    STREAMING.store(false, Ordering::Release);
    SAMPLE_QUEUE.clear();
}

pub fn is_streaming() -> bool {
    STREAMING.load(Ordering::Acquire)
}

pub fn report_drop() {
    DROPPED_BLOCKS.fetch_add(1, Ordering::Relaxed);
}

pub fn dropped_blocks() -> u32 {
    DROPPED_BLOCKS.load(Ordering::Relaxed)
}

pub fn session_id() -> u32 {
    SESSION_ID.load(Ordering::Acquire)
}
