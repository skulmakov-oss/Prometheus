#![no_std]
#![no_main]

use core::arch::asm;
use core::mem::size_of;
use core::panic::PanicInfo;

const SYS_LOG: u64 = 1;
const SYS_TICK: u64 = 2;
const SYS_YIELD: u64 = 3;
const SYS_IPC_SEND: u64 = 4;
const SYS_IPC_RECV: u64 = 5;

const EBUSY: i64 = -16;

const IPC_KIND_PING: u32 = 1;
const IPC_KIND_PONG: u32 = 2;
const IPC_TARGET_PONGS: u32 = 3;

#[repr(C)]
#[derive(Copy, Clone)]
struct Msg {
    kind: u32,
    arg0: u64,
    arg1: u64,
}

const MSG_SIZE: usize = size_of::<Msg>();

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {
        unsafe {
            asm!("hlt", options(nomem, nostack));
        }
    }
}

#[no_mangle]
pub extern "sysv64" fn _start() -> u64 {
    let _ = log_text("hello-sys");

    let t1 = sys_tick();
    let t2 = sys_tick();
    let mut tick_line = [0u8; 96];
    let mut n = 0usize;
    n += append_bytes(&mut tick_line[n..], b"tick t1=");
    n += append_hex_u64(&mut tick_line[n..], t1 as u64);
    n += append_bytes(&mut tick_line[n..], b" t2=");
    n += append_hex_u64(&mut tick_line[n..], t2 as u64);
    let _ = log_bytes(&tick_line[..n]);

    if send_pings() {
        let _ = log_text("ipc ping sent");
    } else {
        let _ = log_text("ipc ping send fail");
        return 2;
    }

    if wait_for_pongs(IPC_TARGET_PONGS) {
        let _ = log_text("pong ok");
        return 0;
    }

    let _ = log_text("pong timeout");
    1
}

fn send_pings() -> bool {
    let mut i = 0u32;
    while i < IPC_TARGET_PONGS {
        let r = sys_ipc_send(IPC_KIND_PING, i as u64, 0xB500 + i as u64);
        if r == EBUSY {
            spin_delay();
            continue;
        }
        if r != 0 {
            return false;
        }
        i += 1;
    }
    true
}

fn wait_for_pongs(target: u32) -> bool {
    let mut got = 0u32;
    let mut msg = Msg {
        kind: 0,
        arg0: 0,
        arg1: 0,
    };

    let mut rounds = 0u32;
    while rounds < 2048 {
        let r = sys_ipc_recv((&mut msg as *mut Msg).cast::<u8>(), MSG_SIZE);
        if r == MSG_SIZE as i64 {
            if msg.kind == IPC_KIND_PONG {
                got += 1;
                if got >= target {
                    return true;
                }
            }
        } else if r != 0 && r != EBUSY {
            return false;
        }

        let _ = sys_yield();
        spin_delay();
        rounds += 1;
    }

    false
}

fn spin_delay() {
    let mut i = 0u32;
    while i < 20000 {
        core::hint::spin_loop();
        i += 1;
    }
}

fn log_text(s: &str) -> i64 {
    log_bytes(s.as_bytes())
}

fn log_bytes(bytes: &[u8]) -> i64 {
    sys_log(bytes.as_ptr(), bytes.len())
}

fn sys_log(ptr: *const u8, len: usize) -> i64 {
    unsafe {
        let ret: u64;
        asm!(
            "int 0x80",
            inlateout("rax") SYS_LOG => ret,
            in("rdi") ptr as u64,
            in("rsi") len as u64,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack)
        );
        ret as i64
    }
}

fn sys_tick() -> i64 {
    unsafe {
        let ret: u64;
        asm!(
            "int 0x80",
            inlateout("rax") SYS_TICK => ret,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack)
        );
        ret as i64
    }
}

fn sys_yield() -> i64 {
    unsafe {
        let ret: u64;
        asm!(
            "int 0x80",
            inlateout("rax") SYS_YIELD => ret,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack)
        );
        ret as i64
    }
}

fn sys_ipc_send(kind: u32, arg0: u64, arg1: u64) -> i64 {
    unsafe {
        let ret: u64;
        asm!(
            "int 0x80",
            inlateout("rax") SYS_IPC_SEND => ret,
            in("rdi") kind as u64,
            in("rsi") arg0,
            in("rdx") arg1,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack)
        );
        ret as i64
    }
}

fn sys_ipc_recv(ptr: *mut u8, len: usize) -> i64 {
    unsafe {
        let ret: u64;
        asm!(
            "int 0x80",
            inlateout("rax") SYS_IPC_RECV => ret,
            in("rdi") ptr as u64,
            in("rsi") len as u64,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack)
        );
        ret as i64
    }
}

fn append_bytes(dst: &mut [u8], src: &[u8]) -> usize {
    let count = core::cmp::min(dst.len(), src.len());
    dst[..count].copy_from_slice(&src[..count]);
    count
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
