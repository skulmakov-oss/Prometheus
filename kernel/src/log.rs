use crate::framebuffer::FramebufferWriter;
use crate::serial;

pub fn both(fb: &mut FramebufferWriter, s: &str) {
    fb.write_line(s);
    serial::line(s);
}

pub fn serial_only(s: &str) {
    serial::line(s);
}
