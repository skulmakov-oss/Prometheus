use core::cell::UnsafeCell;
use core::mem::size_of;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, Ordering};

use crate::interrupts;
use crate::kernel_state::{IpcMsg, IPC_DRAIN_BUDGET, IPC_KIND_PING, IPC_KIND_PONG, IPC_RING_CAP};
use crate::serial;

pub const SYS_LOG: u64 = 1;
pub const SYS_TICK: u64 = 2;
pub const SYS_YIELD: u64 = 3;
pub const SYS_IPC_SEND: u64 = 4;
pub const SYS_IPC_RECV: u64 = 5;

pub const SYS_LOG_MAX: usize = 256;
pub const SYS_LOG_BUDGET_BYTES_PER_TICK: u32 = 64;
pub const SYS_U_BUDGET_SYSCALLS_PER_TICK: u32 = 8;

pub const ERR_EINVAL: i64 = -22;
pub const ERR_EFAULT: i64 = -14;
pub const ERR_ERANGE: i64 = -34;
pub const ERR_EBUSY: i64 = -16;

const IDX_R10: usize = 5;
const IDX_R9: usize = 6;
const IDX_R8: usize = 7;
const IDX_RDI: usize = 9;
const IDX_RSI: usize = 10;
const IDX_RDX: usize = 11;
const IDX_RAX: usize = 14;
const IPC_MSG_SIZE: usize = size_of::<IpcMsg>();

struct MsgRingStorage(UnsafeCell<[IpcMsg; IPC_RING_CAP]>);
unsafe impl Sync for MsgRingStorage {}

static LOG_BUDGET_TICK: AtomicU64 = AtomicU64::new(u64::MAX);
static LOG_BUDGET_USED: AtomicU32 = AtomicU32::new(0);
static SYSCALL_BUDGET_TICK: AtomicU64 = AtomicU64::new(u64::MAX);
static SYSCALL_BUDGET_USED: AtomicU32 = AtomicU32::new(0);
static IPC_DRAIN_TICK: AtomicU64 = AtomicU64::new(u64::MAX);
static IPC_DRAIN_USED: AtomicU32 = AtomicU32::new(0);
static YIELD_LOGGED: AtomicBool = AtomicBool::new(false);

static U2K_BUF: MsgRingStorage = MsgRingStorage(UnsafeCell::new([IpcMsg::EMPTY; IPC_RING_CAP]));
static K2U_BUF: MsgRingStorage = MsgRingStorage(UnsafeCell::new([IpcMsg::EMPTY; IPC_RING_CAP]));
static U2K_HEAD: AtomicU8 = AtomicU8::new(0);
static U2K_TAIL: AtomicU8 = AtomicU8::new(0);
static K2U_HEAD: AtomicU8 = AtomicU8::new(0);
static K2U_TAIL: AtomicU8 = AtomicU8::new(0);

pub fn init() {
    LOG_BUDGET_TICK.store(u64::MAX, Ordering::Relaxed);
    LOG_BUDGET_USED.store(0, Ordering::Relaxed);
    SYSCALL_BUDGET_TICK.store(u64::MAX, Ordering::Relaxed);
    SYSCALL_BUDGET_USED.store(0, Ordering::Relaxed);
    IPC_DRAIN_TICK.store(u64::MAX, Ordering::Relaxed);
    IPC_DRAIN_USED.store(0, Ordering::Relaxed);
    YIELD_LOGGED.store(false, Ordering::Relaxed);

    U2K_HEAD.store(0, Ordering::Relaxed);
    U2K_TAIL.store(0, Ordering::Relaxed);
    K2U_HEAD.store(0, Ordering::Relaxed);
    K2U_TAIL.store(0, Ordering::Relaxed);
}

pub fn dispatch_int80(saved_rsp: *mut u64) {
    let frame = unsafe { core::slice::from_raw_parts_mut(saved_rsp, 15) };

    let now_tick = interrupts::tick();
    if !try_consume_syscall_budget(now_tick) {
        frame[IDX_RAX] = errno(ERR_EBUSY);
        return;
    }

    let sysno = frame[IDX_RAX];
    let a1 = frame[IDX_RDI];
    let a2 = frame[IDX_RSI];
    let a3 = frame[IDX_RDX];
    let a4 = frame[IDX_R10];
    let a5 = frame[IDX_R8];
    let a6 = frame[IDX_R9];

    frame[IDX_RAX] = dispatch(sysno, a1, a2, a3, a4, a5, a6);
}

pub fn process_ipc_budgeted() {
    let now_tick = interrupts::tick();
    while try_consume_ipc_budget(now_tick) {
        let Some(msg) = ring_pop(&U2K_BUF, &U2K_HEAD, &U2K_TAIL) else {
            return;
        };

        if msg.kind == IPC_KIND_PING {
            let reply = IpcMsg {
                kind: IPC_KIND_PONG,
                arg0: msg.arg0,
                arg1: msg.arg1,
            };
            let _ = ring_push(&K2U_BUF, &K2U_HEAD, &K2U_TAIL, reply);
        }
    }
}

fn dispatch(sysno: u64, a1: u64, a2: u64, a3: u64, _a4: u64, _a5: u64, _a6: u64) -> u64 {
    match sysno {
        SYS_LOG => sys_log(a1, a2),
        SYS_TICK => sys_tick(),
        SYS_YIELD => sys_yield(),
        SYS_IPC_SEND => sys_ipc_send(a1, a2, a3),
        SYS_IPC_RECV => sys_ipc_recv(a1, a2),
        _ => errno(ERR_EINVAL),
    }
}

fn sys_log(ptr: u64, len: u64) -> u64 {
    if len == 0 {
        return 0;
    }

    if ptr == 0 || !is_canonical_user_ptr(ptr) {
        return errno(ERR_EFAULT);
    }

    let len_usize = len as usize;
    if len_usize > SYS_LOG_MAX {
        return errno(ERR_ERANGE);
    }

    if ptr.checked_add(len).is_none() {
        return errno(ERR_EFAULT);
    }

    let now_tick = interrupts::tick();
    if !try_consume_log_budget(now_tick, len as u32) {
        return errno(ERR_EBUSY);
    }

    let src = ptr as *const u8;
    let mut payload = [0u8; SYS_LOG_MAX];
    let mut i = 0usize;
    while i < len_usize {
        payload[i] = unsafe { core::ptr::read_volatile(src.add(i)) };
        i += 1;
    }

    let mut out = [0u8; 4 + SYS_LOG_MAX];
    out[0] = b'U';
    out[1] = b'L';
    out[2] = b':';
    out[3] = b' ';
    out[4..(4 + len_usize)].copy_from_slice(&payload[..len_usize]);
    serial::line_bytes(&out[..(4 + len_usize)]);

    len
}

fn sys_tick() -> u64 {
    interrupts::tick()
}

fn sys_yield() -> u64 {
    if !YIELD_LOGGED.swap(true, Ordering::Relaxed) {
        serial::line("UY0 yield noop");
    }
    0
}

fn sys_ipc_send(kind: u64, arg0: u64, arg1: u64) -> u64 {
    if kind == 0 || kind > (u32::MAX as u64) {
        return errno(ERR_EINVAL);
    }

    let msg = IpcMsg {
        kind: kind as u32,
        arg0,
        arg1,
    };

    if !ring_push(&U2K_BUF, &U2K_HEAD, &U2K_TAIL, msg) {
        return errno(ERR_EBUSY);
    }

    0
}

fn sys_ipc_recv(ptr: u64, len: u64) -> u64 {
    if len as usize != IPC_MSG_SIZE {
        return errno(ERR_ERANGE);
    }
    if ptr == 0 || !is_canonical_user_ptr(ptr) {
        return errno(ERR_EFAULT);
    }
    if ptr.checked_add(len).is_none() {
        return errno(ERR_EFAULT);
    }

    process_ipc_budgeted();

    let Some(msg) = ring_pop(&K2U_BUF, &K2U_HEAD, &K2U_TAIL) else {
        return 0;
    };

    let dst = ptr as *mut u8;
    let src = &msg as *const IpcMsg as *const u8;
    unsafe {
        core::ptr::copy_nonoverlapping(src, dst, IPC_MSG_SIZE);
    }

    IPC_MSG_SIZE as u64
}

fn try_consume_syscall_budget(tick: u64) -> bool {
    loop {
        let seen_tick = SYSCALL_BUDGET_TICK.load(Ordering::Relaxed);
        if seen_tick != tick {
            if SYSCALL_BUDGET_TICK
                .compare_exchange(seen_tick, tick, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
            {
                SYSCALL_BUDGET_USED.store(0, Ordering::Relaxed);
            }
            continue;
        }

        let used = SYSCALL_BUDGET_USED.load(Ordering::Relaxed);
        let next = used.saturating_add(1);
        if next > SYS_U_BUDGET_SYSCALLS_PER_TICK {
            return false;
        }

        if SYSCALL_BUDGET_USED
            .compare_exchange(used, next, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
        {
            return true;
        }
    }
}

fn try_consume_ipc_budget(tick: u64) -> bool {
    loop {
        let seen_tick = IPC_DRAIN_TICK.load(Ordering::Relaxed);
        if seen_tick != tick {
            if IPC_DRAIN_TICK
                .compare_exchange(seen_tick, tick, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
            {
                IPC_DRAIN_USED.store(0, Ordering::Relaxed);
            }
            continue;
        }

        let used = IPC_DRAIN_USED.load(Ordering::Relaxed);
        let next = used.saturating_add(1);
        if next > (IPC_DRAIN_BUDGET as u32) {
            return false;
        }

        if IPC_DRAIN_USED
            .compare_exchange(used, next, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
        {
            return true;
        }
    }
}

fn try_consume_log_budget(tick: u64, bytes: u32) -> bool {
    if bytes > SYS_LOG_BUDGET_BYTES_PER_TICK {
        return false;
    }

    loop {
        let seen_tick = LOG_BUDGET_TICK.load(Ordering::Relaxed);
        if seen_tick != tick {
            if LOG_BUDGET_TICK
                .compare_exchange(seen_tick, tick, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
            {
                LOG_BUDGET_USED.store(0, Ordering::Relaxed);
            }
            continue;
        }

        let used = LOG_BUDGET_USED.load(Ordering::Relaxed);
        let next = used.saturating_add(bytes);
        if next > SYS_LOG_BUDGET_BYTES_PER_TICK {
            return false;
        }

        if LOG_BUDGET_USED
            .compare_exchange(used, next, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
        {
            return true;
        }
    }
}

fn ring_push(
    buf: &MsgRingStorage,
    head: &AtomicU8,
    tail: &AtomicU8,
    msg: IpcMsg,
) -> bool {
    let tail_idx = tail.load(Ordering::Relaxed) as usize;
    let next_idx = (tail_idx + 1) % IPC_RING_CAP;
    let head_idx = head.load(Ordering::Acquire) as usize;
    if next_idx == head_idx {
        return false;
    }

    unsafe {
        (*buf.0.get())[tail_idx] = msg;
    }
    tail.store(next_idx as u8, Ordering::Release);
    true
}

fn ring_pop(
    buf: &MsgRingStorage,
    head: &AtomicU8,
    tail: &AtomicU8,
) -> Option<IpcMsg> {
    let head_idx = head.load(Ordering::Relaxed) as usize;
    let tail_idx = tail.load(Ordering::Acquire) as usize;
    if head_idx == tail_idx {
        return None;
    }

    let msg = unsafe { (*buf.0.get())[head_idx] };
    let next_idx = (head_idx + 1) % IPC_RING_CAP;
    head.store(next_idx as u8, Ordering::Release);
    Some(msg)
}

fn is_canonical_user_ptr(ptr: u64) -> bool {
    let sign = (ptr >> 47) & 1;
    let upper = ptr >> 48;
    if sign == 0 {
        upper == 0
    } else {
        upper == 0xFFFF
    }
}

const fn errno(code: i64) -> u64 {
    code as u64
}
