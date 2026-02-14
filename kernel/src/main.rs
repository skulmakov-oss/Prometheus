#![no_std]
#![no_main]

mod demo;
mod font;
mod framebuffer;
mod hal;
mod interrupts;
mod kernel_state;
mod log;
mod quad;
mod runtime;
mod serial;
mod softirq;
mod transjector;
mod transvector;

use core::panic::PanicInfo;
use framebuffer::{Framebuffer, FramebufferWriter};
use kernel_state::{
    KernelState, TaskState, EC_COUNT, EVENT_FRAME_EMPTY, EVT_SLOTS, EVQ_CAP_CLASS, MAX_TASKS,
    MAX_TX, SLEEP_NONE, SRC_NONE,
};

#[repr(C)]
pub struct BootInfo {
    pub magic: u64,
    pub framebuffer: Framebuffer,
    pub memory_map: MemoryMapInfo,
    pub rsdp_addr: u64,
}

#[repr(C)]
pub struct MemoryMapInfo {
    pub map: *const u8,
    pub size: usize,
    pub desc_size: usize,
}

const BOOT_MAGIC: u64 = 0x534F544345565F56;

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    halt()
}

#[no_mangle]
pub extern "sysv64" fn kernel_main(info: *const BootInfo) -> ! {
    serial::init();
    log::serial_only("K00 kernel entry");

    if info.is_null() {
        log::serial_only("E01 bootinfo null");
        halt();
    }

    let bootinfo: &'static BootInfo = unsafe { &*info };
    if bootinfo.magic != BOOT_MAGIC {
        log::serial_only("E02 boot magic");
        halt();
    }
    if bootinfo.framebuffer.base == 0 {
        log::serial_only("E03 fb base");
        halt();
    }
    if bootinfo.framebuffer.stride == 0 {
        log::serial_only("E04 fb stride");
        halt();
    }
    if bootinfo.framebuffer.width == 0 || bootinfo.framebuffer.height == 0 {
        log::serial_only("E05 fb size");
        halt();
    }
    if bootinfo.framebuffer.width > 16384
        || bootinfo.framebuffer.height > 16384
        || bootinfo.framebuffer.stride < bootinfo.framebuffer.width
    {
        log::serial_only("E06 fb bounds");
        halt();
    }
    log::serial_only("K01 bootinfo ok");

    paint_screen(bootinfo.framebuffer, 0x00103070);

    let mut fb = FramebufferWriter::new(bootinfo.framebuffer);
    fb.clear();
    log::both(&mut fb, "K00 kernel entry");
    log::both(&mut fb, "K01 bootinfo ok");
    log::serial_only("K02 framebuffer writer");

    fb.write_line("VectorOS Kernel v0");

    let mut line1 = [0u8; 64];
    let mut n1 = 0usize;
    n1 += append_bytes(&mut line1[n1..], b"Framebuffer: ");
    n1 += append_u64(&mut line1[n1..], bootinfo.framebuffer.width as u64);
    n1 += append_bytes(&mut line1[n1..], b" x ");
    n1 += append_u64(&mut line1[n1..], bootinfo.framebuffer.height as u64);
    if let Ok(s) = core::str::from_utf8(&line1[..n1]) {
        fb.write_line(s);
    }
    log::serial_only("K03 framebuffer info");

    let entries = if bootinfo.memory_map.desc_size == 0 {
        0
    } else {
        bootinfo.memory_map.size / bootinfo.memory_map.desc_size
    };

    let mut line2 = [0u8; 64];
    let mut n2 = 0usize;
    n2 += append_bytes(&mut line2[n2..], b"Memory map entries: ");
    n2 += append_u64(&mut line2[n2..], entries as u64);
    if let Ok(s) = core::str::from_utf8(&line2[..n2]) {
        fb.write_line(s);
    }
    log::serial_only("K04 memory map info");

    demo::run(&mut fb);
    interrupts::init();

    let mut state = KernelState {
        bootinfo,
        fb,
        tick: 0,
        last_irq_tick: 0,
        missed: 0,
        max_dt: 0,
        current_task: 0,
        task_count: 3,
        tasks: [
            TaskState::Ready,
            TaskState::Ready,
            TaskState::Blocked,
            TaskState::Blocked,
            TaskState::Blocked,
            TaskState::Blocked,
            TaskState::Blocked,
            TaskState::Blocked,
        ],
        sleep_until: [SLEEP_NONE; MAX_TASKS],
        wait_mask: [0; MAX_TASKS],
        pending_evt: [0; MAX_TASKS],
        last_wake: [0; MAX_TASKS],
        sub_mask: [0; MAX_TASKS],
        mb_evt: [0; MAX_TASKS],
        mb_cnt: [[0; EVT_SLOTS]; MAX_TASKS],
        mb_payload: [[0; EVT_SLOTS]; MAX_TASKS],
        mb_src: [[SRC_NONE; EVT_SLOTS]; MAX_TASKS],
        mb_tick: [[0; EVT_SLOTS]; MAX_TASKS],
        ovf: [0; MAX_TASKS],
        handled_cnt: [[0; EVT_SLOTS]; MAX_TASKS],
        handled_ovf: [0; MAX_TASKS],
        dispatch_budget: [kernel_state::TASK_DISPATCH_BUDGET; MAX_TASKS],
        dispatch_budget_last_log: 0,
        lat_max: [0; EVT_SLOTS],
        lat_last: [0; EVT_SLOTS],
        lat_viol: [0; EVT_SLOTS],
        dl_last_log_tick: [0; EVT_SLOTS],
        dl_win_viol: [0; EVT_SLOTS],
        dl_win_max: [0; EVT_SLOTS],
        dl_win_start_tick: [0; EVT_SLOTS],
        evq: [[EVENT_FRAME_EMPTY; EVQ_CAP_CLASS]; EC_COUNT],
        evq_head: [0; EC_COUNT],
        evq_tail: [0; EC_COUNT],
        evq_len: [0; EC_COUNT],
        evq_ovf: [0; EC_COUNT],
        evq_starve: [0; EC_COUNT],
        tx_used: [false; MAX_TX],
        tx_enabled: [false; MAX_TX],
        tx_tid: [0; MAX_TX],
        tx_raw_kind_mask: [0; MAX_TX],
        tx_interest_mask: [0; MAX_TX],
        tx_ingest: [None; MAX_TX],
        tx_count: 0,
        woke: 0,
        logic_counter: 0,
        task2_counter: 0,
    };
    runtime::run(&mut state)
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
    for i in 0..out {
        dst[i] = rev[n - 1 - i];
    }
    out
}

fn halt() -> ! {
    loop {
        core::hint::spin_loop();
    }
}

fn paint_screen(fb: Framebuffer, rgb: u32) {
    let width = fb.width as usize;
    let height = fb.height as usize;
    let stride = fb.stride as usize;
    let size = fb.size as usize;
    let (r, g, b) = (
        ((rgb >> 16) & 0xFF) as u8,
        ((rgb >> 8) & 0xFF) as u8,
        (rgb & 0xFF) as u8,
    );

    let mut y = 0usize;
    while y < height {
        let mut x = 0usize;
        while x < width {
            let idx = y.saturating_mul(stride).saturating_add(x);
            let off = idx.saturating_mul(4);
            if off + 3 >= size {
                return;
            }
            unsafe {
                let ptr = (fb.base as *mut u8).add(off);
                if fb.format == 1 {
                    ptr.write_volatile(b);
                    ptr.add(1).write_volatile(g);
                    ptr.add(2).write_volatile(r);
                } else {
                    ptr.write_volatile(r);
                    ptr.add(1).write_volatile(g);
                    ptr.add(2).write_volatile(b);
                }
                ptr.add(3).write_volatile(0);
            }
            x += 1;
        }
        y += 1;
    }
}
