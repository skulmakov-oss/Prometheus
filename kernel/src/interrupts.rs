use core::arch::asm;
use core::sync::atomic::{AtomicU64, Ordering};

use crate::log;
use crate::softirq::{self, SOFTIRQ_SCHED};
use crate::syscall;

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

const GPR_SAVE_SLOTS: usize = 15;
const IDX_ERR: usize = GPR_SAVE_SLOTS;
const IDX_RIP: usize = GPR_SAVE_SLOTS + 1;
const IDX_RFLAGS: usize = GPR_SAVE_SLOTS + 3;

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
    ".global int80_stub",
    "int80_stub:",
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
    "mov rdi, rsp",
    "call int80_rust",
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
    ".global ud_stub",
    "ud_stub:",
    "push 0",
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
    "mov rdi, rsp",
    "mov rsi, [rsp + 120]",
    "call exc_ud_rust",
    ".global gp_stub",
    "gp_stub:",
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
    "mov rdi, rsp",
    "mov rsi, [rsp + 120]",
    "call exc_gp_rust",
    ".global pf_stub",
    "pf_stub:",
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
    "mov rdi, rsp",
    "mov rsi, [rsp + 120]",
    "call exc_pf_rust",
    ".global df_stub",
    "df_stub:",
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
    "mov rdi, rsp",
    "mov rsi, [rsp + 120]",
    "call exc_df_rust",
);

unsafe extern "C" {
    fn irq0_stub();
    fn int80_stub();
    fn ud_stub();
    fn gp_stub();
    fn pf_stub();
    fn df_stub();
}

pub fn init() {
    unsafe {
        let code_selector = current_cs();
        idt_set_handler(6, ud_stub as *const () as usize as u64, code_selector);
        idt_set_handler(8, df_stub as *const () as usize as u64, code_selector);
        idt_set_handler(13, gp_stub as *const () as usize as u64, code_selector);
        idt_set_handler(14, pf_stub as *const () as usize as u64, code_selector);
        idt_set_handler(32, irq0_stub as *const () as usize as u64, code_selector);
        idt_set_handler_user(0x80, int80_stub as *const () as usize as u64, code_selector);

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

#[unsafe(no_mangle)]
extern "C" fn int80_rust(saved_rsp: *mut u64) {
    syscall::dispatch_int80(saved_rsp);
}

#[unsafe(no_mangle)]
extern "C" fn exc_ud_rust(saved_rsp: *const u64, err: u64) -> ! {
    exception_fatal("XUD", 6, err, saved_rsp, None)
}

#[unsafe(no_mangle)]
extern "C" fn exc_gp_rust(saved_rsp: *const u64, err: u64) -> ! {
    exception_fatal("XGP", 13, err, saved_rsp, None)
}

#[unsafe(no_mangle)]
extern "C" fn exc_pf_rust(saved_rsp: *const u64, err: u64) -> ! {
    let cr2 = read_cr2();
    exception_fatal("XPF", 14, err, saved_rsp, Some(cr2))
}

#[unsafe(no_mangle)]
extern "C" fn exc_df_rust(saved_rsp: *const u64, err: u64) -> ! {
    exception_fatal("XDF", 8, err, saved_rsp, None)
}

fn exception_fatal(tag: &str, vector: u8, err: u64, saved_rsp: *const u64, cr2: Option<u64>) -> ! {
    log::set_marker(0x5800 + vector as u32);

    let lm = log::last_marker() as u64;
    let rip = unsafe { *saved_rsp.add(IDX_RIP) };
    let rflags = unsafe { *saved_rsp.add(IDX_RFLAGS) };
    let frame_rsp = (saved_rsp as u64).wrapping_add((IDX_ERR as u64) * 8);

    let mut l1 = [0u8; 96];
    let mut n1 = 0usize;
    n1 += append_bytes(&mut l1[n1..], tag.as_bytes());
    n1 += append_bytes(&mut l1[n1..], b" v=");
    n1 += append_u64(&mut l1[n1..], vector as u64);
    n1 += append_bytes(&mut l1[n1..], b" e=");
    n1 += append_hex_u64(&mut l1[n1..], err);
    n1 += append_bytes(&mut l1[n1..], b" lm=");
    n1 += append_hex_u64(&mut l1[n1..], lm);
    if let Ok(s) = core::str::from_utf8(&l1[..n1]) {
        let _ = log::serial_try(s);
    }

    let mut l2 = [0u8; 128];
    let mut n2 = 0usize;
    n2 += append_bytes(&mut l2[n2..], b"XD1 rip=");
    n2 += append_hex_u64(&mut l2[n2..], rip);
    n2 += append_bytes(&mut l2[n2..], b" rsp=");
    n2 += append_hex_u64(&mut l2[n2..], frame_rsp);
    n2 += append_bytes(&mut l2[n2..], b" rfl=");
    n2 += append_hex_u64(&mut l2[n2..], rflags);
    if let Ok(s) = core::str::from_utf8(&l2[..n2]) {
        let _ = log::serial_try(s);
    }

    if let Some(cr2v) = cr2 {
        let mut l3 = [0u8; 48];
        let mut n3 = 0usize;
        n3 += append_bytes(&mut l3[n3..], b"XPF cr2=");
        n3 += append_hex_u64(&mut l3[n3..], cr2v);
        if let Ok(s) = core::str::from_utf8(&l3[..n3]) {
            let _ = log::serial_try(s);
        }
    }

    halt_loop()
}

#[cfg(feature = "exc_df")]
pub fn trigger_df_acceptance() -> ! {
    let mut frame = [0u64; IDX_RFLAGS + 1];
    frame[IDX_ERR] = 0;
    frame[IDX_RIP] = read_rip();
    frame[IDX_RFLAGS] = read_rflags();
    exception_fatal("XDF", 8, 0, frame.as_ptr(), None)
}

#[cfg(feature = "exc_df")]
fn read_rip() -> u64 {
    let value: u64;
    unsafe {
        asm!("lea {}, [rip]", out(reg) value, options(nomem, nostack, preserves_flags));
    }
    value
}

#[cfg(feature = "exc_df")]
fn read_rflags() -> u64 {
    let value: u64;
    unsafe {
        asm!("pushfq", "pop {}", out(reg) value, options(nomem, preserves_flags));
    }
    value
}

fn read_cr2() -> u64 {
    let value: u64;
    unsafe {
        asm!("mov {}, cr2", out(reg) value, options(nomem, nostack, preserves_flags));
    }
    value
}

pub fn halt_forever() -> ! {
    halt_loop()
}

fn halt_loop() -> ! {
    loop {
        unsafe {
            asm!("cli", options(nomem, nostack, preserves_flags));
            asm!("hlt", options(nomem, nostack));
        }
        core::hint::spin_loop();
    }
}

unsafe fn idt_set_handler(vector: usize, handler: u64, selector: u16) {
    idt_set_handler_with_options(vector, handler, selector, 0x8E00);
}

unsafe fn idt_set_handler_user(vector: usize, handler: u64, selector: u16) {
    idt_set_handler_with_options(vector, handler, selector, 0xEE00);
}

unsafe fn idt_set_handler_with_options(vector: usize, handler: u64, selector: u16, options: u16) {
    let entry = &mut IDT[vector];
    entry.offset_low = (handler & 0xFFFF) as u16;
    entry.selector = selector;
    entry.options = options;
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

fn append_bytes(dst: &mut [u8], src: &[u8]) -> usize {
    let count = core::cmp::min(dst.len(), src.len());
    dst[..count].copy_from_slice(&src[..count]);
    count
}

fn append_u64(dst: &mut [u8], mut value: u64) -> usize {
    if dst.is_empty() {
        return 0;
    }
    if value == 0 {
        dst[0] = b'0';
        return 1;
    }
    let mut rev = [0u8; 20];
    let mut n = 0usize;
    while value > 0 && n < rev.len() {
        rev[n] = b'0' + (value % 10) as u8;
        value /= 10;
        n += 1;
    }
    let out = core::cmp::min(n, dst.len());
    let mut i = 0usize;
    while i < out {
        dst[i] = rev[n - 1 - i];
        i += 1;
    }
    out
}

fn append_hex_u64(dst: &mut [u8], mut value: u64) -> usize {
    if dst.is_empty() {
        return 0;
    }
    if value == 0 {
        dst[0] = b'0';
        return 1;
    }
    let mut rev = [0u8; 16];
    let mut n = 0usize;
    while value > 0 && n < rev.len() {
        let d = (value & 0xF) as u8;
        rev[n] = if d < 10 { b'0' + d } else { b'a' + (d - 10) };
        value >>= 4;
        n += 1;
    }
    let out = core::cmp::min(n, dst.len());
    let mut i = 0usize;
    while i < out {
        dst[i] = rev[n - 1 - i];
        i += 1;
    }
    out
}
