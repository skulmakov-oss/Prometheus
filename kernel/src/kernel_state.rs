use crate::framebuffer::FramebufferWriter;
use crate::BootInfo;

pub type TaskId = u8;
pub const MAX_TASKS: usize = 8;
pub const SLEEP_NONE: u64 = u64::MAX;
#[allow(dead_code)]
pub const EVT_NONE: u32 = 0;
#[allow(dead_code)]
pub const EVT_TIMER: u32 = 1 << 0;
pub const EVT_BUS: u32 = 1 << 1;
pub const EVT_FAB: u32 = 1 << 2;
pub const EVT_LOG: u32 = 1 << 3;
#[allow(dead_code)]
pub const EVT_ALL: u32 = EVT_TIMER | EVT_BUS | EVT_FAB | EVT_LOG;
pub const EVT_SLOTS: usize = 4;
pub const SLOT_TIMER: usize = 0;
pub const SLOT_BUS: usize = 1;
pub const SLOT_FAB: usize = 2;
pub const SLOT_LOG: usize = 3;
pub const SRC_NONE: u8 = 0;
pub const SRC_SOFTIRQ: u8 = 1;
#[allow(dead_code)]
pub const SRC_TIMER: u8 = 2;
#[allow(dead_code)]
pub const SRC_DRIVER: u8 = 3;
pub const SRC_TASK: u8 = 4;
#[allow(dead_code)]
pub const SRC_HAL: u8 = 5;
#[allow(dead_code)]
pub const SRC_TX: u8 = 6;
pub const MAX_TX: usize = 8;

pub const RAW_SOFTIRQ: u8 = 1;
pub const RAW_IRQ: u8 = 2;
pub const RAW_MMIO: u8 = 3;
pub const RAW_INTERNAL: u8 = 4;

pub const RAWC_SOFTIRQ_TIMER: u16 = 0x0101;
pub const RAWC_SOFTIRQ_BUS: u16 = 0x0110;
pub const RAWC_SOFTIRQ_FABRIC: u16 = 0x0120;
pub const RAWC_SOFTIRQ_LOG: u16 = 0x0130;
pub const RAWC_IRQ_GRP_TIMER: u16 = 0x0200;
pub const RAWC_IRQ_GRP_BUS: u16 = 0x0210;
pub const RAWC_IRQ_GRP_FAB: u16 = 0x0220;
pub const RAWC_MMIO_BUS_RX: u16 = 0x0310;
pub const RAWC_MMIO_BUS_TX: u16 = 0x0311;
pub const RAWC_MMIO_FAB_ALERT: u16 = 0x0320;
pub const RAWC_INTERNAL_LOG_FLUSH: u16 = 0x0401;
pub const RAWC_INTERNAL_PANIC: u16 = 0x04F0;
pub const EC_CRIT: usize = 0;
pub const EC_HIGH: usize = 1;
pub const EC_NORM: usize = 2;
pub const EC_LOW: usize = 3;
pub const EC_COUNT: usize = 4;
pub const EVQ_CAP_CLASS: usize = 16;
pub const EVQ_DRAIN_BUDGET: u16 = 8;
pub const EVQ_DRAIN_QUOTA: [u16; EC_COUNT] = [2, 2, 2, 2];
pub const EVQ_STARVE_WARN_TICKS: u16 = 64;
pub const TASK_DISPATCH_BUDGET: u8 = 4;
pub const TASK_DISPATCH_MAX_SLOTS_PER_TICK: u8 = 2;
pub const TASK_DISPATCH_BUDGET_MIN: u8 = 2;
pub const TASK_DISPATCH_BUDGET_MAX: u8 = 8;
pub const CRIT_DISPATCH_BUDGET: u8 = 2;
pub const CRIT_DISPATCH_MAX_SLOTS_PER_TICK: u8 = 1;
pub const DL_TIMER: u16 = 2;
pub const DL_FAB: u16 = 4;
pub const DL_BUS: u16 = 8;
pub const DL_LOG: u16 = 32;
pub const DL_TABLE: [u16; EVT_SLOTS] = [DL_TIMER, DL_BUS, DL_FAB, DL_LOG];
pub const DL_LOG_THROTTLE_TICKS: u32 = 64;
pub const DL_LOG_WINDOW_TICKS: u32 = 256;
pub const DL_LOG_MIN_PERIOD_TICKS: u32 = 64;

#[derive(Copy, Clone, Eq, PartialEq)]
pub enum TaskState {
    Ready,
    Blocked,
}

#[derive(Copy, Clone)]
pub struct EventFrame {
    pub evt_mask: u32,
    pub src_id: u8,
    pub tick: u32,
    pub payload: u32,
}

#[derive(Copy, Clone)]
pub struct RawEvent {
    pub src: u8,
    pub kind: u8,
    pub code: u16,
    pub arg: u32,
}

#[derive(Copy, Clone)]
pub struct VectorEvent {
    pub evt_mask: u32,
    pub src_id: u8,
    pub payload: u32,
}

pub type TxIngestFn = fn(&mut KernelState, RawEvent) -> Option<VectorEvent>;

pub const EVENT_FRAME_EMPTY: EventFrame = EventFrame {
    evt_mask: 0,
    src_id: SRC_NONE,
    tick: 0,
    payload: 0,
};

pub struct KernelState {
    pub bootinfo: &'static BootInfo,
    pub fb: FramebufferWriter,
    pub tick: u64,
    pub last_irq_tick: u64,
    pub missed: u64,
    pub max_dt: u64,
    pub current_task: TaskId,
    pub task_count: u8,
    pub tasks: [TaskState; MAX_TASKS],
    pub sleep_until: [u64; MAX_TASKS],
    pub wait_mask: [u32; MAX_TASKS],
    pub pending_evt: [u32; MAX_TASKS],
    pub last_wake: [u32; MAX_TASKS],
    pub sub_mask: [u32; MAX_TASKS],
    pub mb_evt: [u32; MAX_TASKS],
    pub mb_cnt: [[u8; EVT_SLOTS]; MAX_TASKS],
    pub mb_payload: [[u32; EVT_SLOTS]; MAX_TASKS],
    pub mb_src: [[u8; EVT_SLOTS]; MAX_TASKS],
    pub mb_tick: [[u32; EVT_SLOTS]; MAX_TASKS],
    pub ovf: [u32; MAX_TASKS],
    pub handled_cnt: [[u16; EVT_SLOTS]; MAX_TASKS],
    pub handled_ovf: [u16; MAX_TASKS],
    pub dispatch_budget: [u8; MAX_TASKS],
    pub dispatch_budget_last_log: u32,
    pub lat_max: [u16; EVT_SLOTS],
    pub lat_last: [u16; EVT_SLOTS],
    pub lat_viol: [u16; EVT_SLOTS],
    pub dl_last_log_tick: [u32; EVT_SLOTS],
    pub dl_win_viol: [u16; EVT_SLOTS],
    pub dl_win_max: [u16; EVT_SLOTS],
    pub dl_win_start_tick: [u32; EVT_SLOTS],
    pub evq: [[EventFrame; EVQ_CAP_CLASS]; EC_COUNT],
    pub evq_head: [u16; EC_COUNT],
    pub evq_tail: [u16; EC_COUNT],
    pub evq_len: [u16; EC_COUNT],
    pub evq_ovf: [u32; EC_COUNT],
    pub evq_starve: [u16; EC_COUNT],
    pub tx_used: [bool; MAX_TX],
    pub tx_enabled: [bool; MAX_TX],
    pub tx_tid: [u8; MAX_TX],
    pub tx_raw_kind_mask: [u32; MAX_TX],
    pub tx_interest_mask: [u32; MAX_TX],
    pub tx_ingest: [Option<TxIngestFn>; MAX_TX],
    pub tx_count: u8,
    pub woke: u64,
    pub logic_counter: u64,
    pub task2_counter: u64,
}
