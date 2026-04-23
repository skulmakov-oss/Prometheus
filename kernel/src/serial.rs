use core::arch::asm;
use core::sync::atomic::{AtomicU64, Ordering};

const COM1: u16 = 0x3F8;
const TX_SPIN_LIMIT: u32 = 100_000;

// Stage 2 policy: fail-open by dropping bytes when TX is not ready in bounded time.
#[allow(dead_code)]
pub const SERIAL_DROP_POLICY: &str = "drop_byte";

static SERIAL_TX_DROPS: AtomicU64 = AtomicU64::new(0);

#[inline(always)]
unsafe fn outb(port: u16, val: u8) {
    unsafe {
        asm!("out dx, al", in("dx") port, in("al") val);
    }
}

#[inline(always)]
unsafe fn inb(port: u16) -> u8 {
    let mut v: u8;
    unsafe {
        asm!("in al, dx", out("al") v, in("dx") port);
    }
    v
}

#[inline(always)]
fn tx_ready() -> bool {
    unsafe { (inb(COM1 + 5) & 0x20) != 0 }
}

pub fn init() {
    unsafe {
        outb(COM1 + 1, 0x00);
        outb(COM1 + 3, 0x80);
        outb(COM1 + 0, 0x03);
        outb(COM1 + 1, 0x00);
        outb(COM1 + 3, 0x03);
        outb(COM1 + 2, 0xC7);
        outb(COM1 + 4, 0x0B);
    }
}

pub fn line(s: &str) {
    line_bytes(s.as_bytes());
}

pub fn line_bytes(bytes: &[u8]) {
    for &b in bytes {
        let _ = write_byte_bounded(b);
    }
    let _ = write_byte_bounded(b'\r');
    let _ = write_byte_bounded(b'\n');
}

// Non-blocking write path for panic/exception diagnostics.
pub fn try_write_line(s: &str) -> bool {
    let mut ok = true;
    for b in s.bytes() {
        ok &= try_write_byte(b);
    }
    ok &= try_write_byte(b'\r');
    ok &= try_write_byte(b'\n');
    ok
}

#[allow(dead_code)]
pub fn tx_drop_count() -> u64 {
    SERIAL_TX_DROPS.load(Ordering::Relaxed)
}

fn try_write_byte(b: u8) -> bool {
    if !tx_ready() {
        SERIAL_TX_DROPS.fetch_add(1, Ordering::Relaxed);
        return false;
    }
    unsafe {
        outb(COM1, b);
    }
    true
}

fn write_byte_bounded(b: u8) -> bool {
    let mut spins = 0u32;
    while !tx_ready() {
        spins = spins.saturating_add(1);
        if spins >= TX_SPIN_LIMIT {
            SERIAL_TX_DROPS.fetch_add(1, Ordering::Relaxed);
            return false;
        }
        core::hint::spin_loop();
    }
    unsafe {
        outb(COM1, b);
    }
    true
}
