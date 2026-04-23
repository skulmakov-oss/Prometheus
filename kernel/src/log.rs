use core::sync::atomic::{AtomicU32, Ordering};

use crate::framebuffer::FramebufferWriter;
use crate::serial;

static LAST_MARKER: AtomicU32 = AtomicU32::new(0);

pub fn both(fb: &mut FramebufferWriter, s: &str) {
    fb.write_line(s);
    serial::line(s);
}

pub fn serial_only(s: &str) {
    serial::line(s);
}

pub fn serial_try(s: &str) -> bool {
    serial::try_write_line(s)
}

pub fn set_marker(marker: u32) {
    LAST_MARKER.store(marker, Ordering::Relaxed);
}

pub fn last_marker() -> u32 {
    LAST_MARKER.load(Ordering::Relaxed)
}

