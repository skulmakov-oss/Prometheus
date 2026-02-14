use crate::interrupts;
use crate::serial;

#[inline(always)]
pub fn time_now_tick() -> u64 {
    interrupts::tick()
}

#[allow(dead_code)]
#[inline(always)]
pub fn irq_ack_timer() {
    interrupts::irq_ack_timer()
}

#[allow(dead_code)]
#[inline(always)]
pub fn serial_write_line(s: &str) {
    serial::line(s);
}

#[allow(dead_code)]
#[inline(always)]
pub unsafe fn mmio_read32(addr: u64) -> u32 {
    unsafe { (addr as *const u32).read_volatile() }
}

#[allow(dead_code)]
#[inline(always)]
pub unsafe fn mmio_write32(addr: u64, v: u32) {
    unsafe { (addr as *mut u32).write_volatile(v) }
}

#[allow(dead_code)]
#[inline(always)]
pub unsafe fn framebuffer_putpixel(base: u64, stride: u32, x: u32, y: u32, rgb: u32) {
    let idx = (y as usize)
        .saturating_mul(stride as usize)
        .saturating_add(x as usize);
    let off = idx.saturating_mul(4);
    let (r, g, b) = (
        ((rgb >> 16) & 0xFF) as u8,
        ((rgb >> 8) & 0xFF) as u8,
        (rgb & 0xFF) as u8,
    );
    unsafe {
        let ptr = (base as *mut u8).add(off);
        ptr.write_volatile(r);
        ptr.add(1).write_volatile(g);
        ptr.add(2).write_volatile(b);
        ptr.add(3).write_volatile(0);
    }
}
