use core::sync::atomic::{AtomicU32, Ordering};

pub const SOFTIRQ_SCHED: u32 = 1 << 0;
pub const SOFTIRQ_BUS: u32 = 1 << 1;
pub const SOFTIRQ_FABRIC: u32 = 1 << 2;
pub const SOFTIRQ_LOG: u32 = 1 << 3;

static SOFTIRQ_PENDING: AtomicU32 = AtomicU32::new(0);

pub fn raise(mask: u32) {
    SOFTIRQ_PENDING.fetch_or(mask, Ordering::Release);
}

pub fn take_pending() -> u32 {
    SOFTIRQ_PENDING.swap(0, Ordering::AcqRel)
}
