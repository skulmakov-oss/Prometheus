use core::arch::asm;

const COM1: u16 = 0x3F8;

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
    for b in s.bytes() {
        write_byte(b);
    }
    write_byte(b'\r');
    write_byte(b'\n');
}

fn write_byte(b: u8) {
    unsafe {
        while (inb(COM1 + 5) & 0x20) == 0 {}
        outb(COM1, b);
    }
}
