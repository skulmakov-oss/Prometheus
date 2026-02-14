use core::arch::asm;
use core::sync::atomic::{AtomicU64, Ordering};

use crate::softirq::{self, SOFTIRQ_SCHED};

#[repr(C, packed)]
#[derive(Copy, Clone)]
struct IdtEntry {
    offset_low: u16,
    selector: u16,
    options: u16,
    offset_mid: u16,
    offset_high: u32,
    reserved: u32,
}

impl IdtEntry {
    const fn missing() -> Self {
        Self {
            offset_low: 0,
            selector: 0,
            options: 0,
            offset_mid: 0,
            offset_high: 0,
            reserved: 0,
        }
    }
}

#[repr(C, packed)]
struct Idtr {
    limit: u16,
    base: u64,
}

static mut IDT: [IdtEntry; 256] = [IdtEntry::missing(); 256];
static GLOBAL_TICK: AtomicU64 = AtomicU64::new(0);

core::arch::global_asm!(
    ".global irq0_stub",
    "irq0_stub:",
    "push rax",
    "push rbx",
    "push rcx",
    "push rdx",
    "push rsi",
    "push rdi",
    "push rbp",
    "push r8",
    "push r9",
    "push r10",
    "push r11",
    "push r12",
    "push r13",
    "push r14",
    "push r15",
    "call irq0_rust",
    "pop r15",
    "pop r14",
    "pop r13",
    "pop r12",
    "pop r11",
    "pop r10",
    "pop r9",
    "pop r8",
    "pop rbp",
    "pop rdi",
    "pop rsi",
    "pop rdx",
    "pop rcx",
    "pop rbx",
    "pop rax",
    "iretq",
);

unsafe extern "C" {
    fn irq0_stub();
}

pub fn init() {
    unsafe {
        let code_selector = current_cs();
        idt_set_handler(32, irq0_stub as *const () as usize as u64, code_selector);
        let idtr = Idtr {
            limit: (core::mem::size_of::<[IdtEntry; 256]>() - 1) as u16,
            base: core::ptr::addr_of!(IDT) as *const _ as u64,
        };
        asm!("lidt [{}]", in(reg) &idtr, options(readonly, nostack));

        pic_remap(0x20, 0x28);
        pit_init_100hz();
        asm!("sti", options(nomem, nostack, preserves_flags));
    }
}

pub fn tick() -> u64 {
    GLOBAL_TICK.load(Ordering::Acquire)
}

pub fn irq_ack_timer() {
    unsafe {
        outb(0x20, 0x20);
    }
}

#[unsafe(no_mangle)]
extern "C" fn irq0_rust() {
    GLOBAL_TICK.fetch_add(1, Ordering::Relaxed);
    softirq::raise(SOFTIRQ_SCHED);
    irq_ack_timer();
}

unsafe fn idt_set_handler(vector: usize, handler: u64, selector: u16) {
    let entry = &mut IDT[vector];
    entry.offset_low = (handler & 0xFFFF) as u16;
    entry.selector = selector;
    entry.options = 0x8E00;
    entry.offset_mid = ((handler >> 16) & 0xFFFF) as u16;
    entry.offset_high = ((handler >> 32) & 0xFFFF_FFFF) as u32;
    entry.reserved = 0;
}

unsafe fn current_cs() -> u16 {
    let cs: u16;
    asm!("mov {0:x}, cs", out(reg) cs, options(nomem, nostack, preserves_flags));
    cs
}

unsafe fn pic_remap(master_offset: u8, slave_offset: u8) {
    let master_mask = inb(0x21);
    let slave_mask = inb(0xA1);

    outb(0x20, 0x11);
    outb(0xA0, 0x11);
    outb(0x21, master_offset);
    outb(0xA1, slave_offset);
    outb(0x21, 0x04);
    outb(0xA1, 0x02);
    outb(0x21, 0x01);
    outb(0xA1, 0x01);

    outb(0x21, master_mask & !0x01);
    outb(0xA1, slave_mask);
}

unsafe fn pit_init_100hz() {
    let div: u16 = 11931;
    outb(0x43, 0x36);
    outb(0x40, (div & 0x00FF) as u8);
    outb(0x40, (div >> 8) as u8);
}

unsafe fn outb(port: u16, val: u8) {
    asm!("out dx, al", in("dx") port, in("al") val, options(nomem, nostack, preserves_flags));
}

unsafe fn inb(port: u16) -> u8 {
    let mut value: u8;
    asm!("in al, dx", out("al") value, in("dx") port, options(nomem, nostack, preserves_flags));
    value
}
