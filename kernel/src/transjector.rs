use crate::quad::QuadReg;
use crate::kernel_state::{
    KernelState, RawEvent, VectorEvent, TxIngestFn, EVT_BUS, EVT_FAB, EVT_LOG, EVT_TIMER, MAX_TX,
    RAW_INTERNAL, RAW_IRQ, RAW_MMIO, RAW_SOFTIRQ, RAWC_INTERNAL_LOG_FLUSH, RAWC_INTERNAL_PANIC,
    RAWC_IRQ_GRP_BUS, RAWC_IRQ_GRP_FAB, RAWC_IRQ_GRP_TIMER, RAWC_MMIO_BUS_RX, RAWC_MMIO_BUS_TX,
    RAWC_MMIO_FAB_ALERT, RAWC_SOFTIRQ_BUS, RAWC_SOFTIRQ_FABRIC, RAWC_SOFTIRQ_LOG, RAWC_SOFTIRQ_TIMER,
};
use crate::log;
use crate::softirq;
use crate::transvector::{read_by_id, write_by_id};

#[derive(Copy, Clone)]
pub struct Transjector {
    pub id: u64,
    pub lane: u8,
    pub state: QuadReg,
}

#[derive(Copy, Clone)]
pub struct TransjectorEvent {
    pub out_id: u64,
    pub t: Transjector,
}

#[derive(Copy, Clone)]
pub struct Lane {
    pub buf: [Option<TransjectorEvent>; 8],
    pub head: u8,
    pub tail: u8,
}

const EMPTY_LANE: Lane = Lane {
    buf: [None; 8],
    head: 0,
    tail: 0,
};

static mut LANES: [Lane; 4] = [EMPTY_LANE; 4];
static mut NEXT_LANE: u8 = 0;
static mut TRACE_TICK: u32 = 0;
static mut TRACE_LAST: TraceRecord = TraceRecord {
    tick: 0,
    lane: 0,
    out_id: 0,
    in_id: 0,
    cur: 0,
    incoming: 0,
    result: 2,
};

#[repr(C)]
#[derive(Copy, Clone)]
pub struct TraceRecord {
    pub tick: u32,
    pub lane: u8,
    pub out_id: u64,
    pub in_id: u64,
    pub cur: u8,
    pub incoming: u8,
    pub result: u8,
}

#[derive(Copy, Clone)]
pub enum StepResult {
    Applied,
    Blocked,
    Empty,
}

const ALLOW: [[bool; 4]; 4] = [
    [true, true, true, true],
    [true, false, true, false],
    [true, true, false, false],
    [false, false, false, true],
];

pub fn transject(id: u64, lane: u8, raw: u64) -> Transjector {
    let state = match raw {
        0 => QuadReg::N,
        1 => QuadReg::T,
        2 => QuadReg::F,
        3 => QuadReg::S,
        _ => QuadReg::N,
    };
    Transjector { id, lane, state }
}

pub fn apply(out_id: u64, t: Transjector) -> bool {
    write_by_id(out_id, t.state)
}

pub fn bus_push(lane: u8, ev: TransjectorEvent) -> bool {
    if lane >= 4 {
        return false;
    }
    unsafe {
        let l = &mut LANES[lane as usize];
        if lane_is_full(l) {
            return false;
        }
        l.buf[l.tail as usize] = Some(ev);
        l.tail = (l.tail + 1) & 7;
        true
    }
}

#[allow(dead_code)]
pub fn bus_step() -> Option<TransjectorEvent> {
    if let Some((_, ev)) = pop_next_event_rr() {
        let _ = apply(ev.out_id, ev.t);
        return Some(ev);
    }
    None
}

pub fn bus_step_gated() -> StepResult {
    let Some((lane, ev)) = pop_next_event_rr() else {
        write_trace(0, 0, 0, 0, 0, 2);
        return StepResult::Empty;
    };

    let cur_bits = read_by_id(ev.out_id)
        .map(|tv| tv.state.bits())
        .unwrap_or(QuadReg::N.bits())
        & 0b11;
    let incoming_bits = ev.t.state.bits() & 0b11;
    if ALLOW[cur_bits as usize][incoming_bits as usize] {
        let _ = apply(ev.out_id, ev.t);
        write_trace(lane, ev.out_id, ev.t.id, cur_bits, incoming_bits, 0);
        StepResult::Applied
    } else {
        write_trace(lane, ev.out_id, ev.t.id, cur_bits, incoming_bits, 1);
        StepResult::Blocked
    }
}

pub fn trace_last() -> TraceRecord {
    unsafe { TRACE_LAST }
}

pub fn bus_pending() -> u32 {
    let mut n = 0u32;
    unsafe {
        let mut li = 0usize;
        while li < 4 {
            let lane = &LANES[li];
            let mut bi = 0usize;
            while bi < 8 {
                if lane.buf[bi].is_some() {
                    n += 1;
                }
                bi += 1;
            }
            li += 1;
        }
    }
    n
}

fn lane_pop(idx: usize) -> Option<TransjectorEvent> {
    let lane = unsafe { &mut LANES[idx] };
    if lane_is_empty(lane) {
        return None;
    }
    let out = lane.buf[lane.head as usize].take();
    lane.head = (lane.head + 1) & 7;
    out
}

fn pop_next_event_rr() -> Option<(u8, TransjectorEvent)> {
    unsafe {
        let start = NEXT_LANE as usize;
        let mut off = 0usize;
        while off < 4 {
            let idx = (start + off) & 3;
            if let Some(ev) = lane_pop(idx) {
                NEXT_LANE = ((idx + 1) & 3) as u8;
                return Some((idx as u8, ev));
            }
            off += 1;
        }
        NEXT_LANE = (NEXT_LANE + 1) & 3;
    }
    None
}

fn write_trace(lane: u8, out_id: u64, in_id: u64, cur: u8, incoming: u8, result: u8) {
    unsafe {
        TRACE_TICK = TRACE_TICK.wrapping_add(1);
        TRACE_LAST = TraceRecord {
            tick: TRACE_TICK,
            lane,
            out_id,
            in_id,
            cur,
            incoming,
            result,
        };
    }
}

fn lane_is_empty(l: &Lane) -> bool {
    l.head == l.tail && l.buf[l.head as usize].is_none()
}

fn lane_is_full(l: &Lane) -> bool {
    l.head == l.tail && l.buf[l.head as usize].is_some()
}

pub const PTYPE_BUS: u8 = 1;
pub const PTYPE_FAB: u8 = 2;
pub const PTYPE_LOG: u8 = 3;
pub const PTYPE_TIMER: u8 = 4;
pub const TIO_TIMER: u8 = 0;
pub const TIO_BUS_SYS: u8 = 1;
pub const TIO_BUS_RX: u8 = 2;
pub const TIO_BUS_TX: u8 = 3;
pub const TIO_FAB_CTRL: u8 = 4;
pub const TIO_FAB_ALERT: u8 = 5;
pub const TIO_LOG_TELEM: u8 = 6;
pub const TIO_LOG_PANIC: u8 = 7;

pub fn pack_payload(ptype: u8, chan: u8, id: u8, sev: u8, flags: u8) -> u32 {
    (ptype as u32)
        | ((chan as u32) << 8)
        | ((id as u32) << 16)
        | (((sev & 0x0F) as u32) << 24)
        | (((flags & 0x0F) as u32) << 28)
}

#[allow(dead_code)]
pub fn payload_set_chan(payload: u32, chan: u8) -> u32 {
    (payload & !(0xFFu32 << 8)) | ((chan as u32) << 8)
}

pub fn payload_get_chan(payload: u32) -> u8 {
    ((payload >> 8) & 0xFF) as u8
}

pub fn raw_kind_bit(kind: u8) -> u32 {
    if kind == 0 || kind > 31 {
        0
    } else {
        1u32 << ((kind - 1) as u32)
    }
}

pub fn tx_register(
    state: &mut KernelState,
    slot: usize,
    tid: u8,
    raw_kind_mask: u32,
    interest_mask: u32,
    ingest_fn: TxIngestFn,
) -> bool {
    if slot >= MAX_TX {
        return false;
    }
    state.tx_used[slot] = true;
    state.tx_enabled[slot] = true;
    state.tx_tid[slot] = tid;
    state.tx_raw_kind_mask[slot] = raw_kind_mask;
    state.tx_interest_mask[slot] = interest_mask;
    state.tx_ingest[slot] = Some(ingest_fn);
    if (slot as u8) >= state.tx_count {
        state.tx_count = slot as u8 + 1;
    }
    log_tx_reg(tid, slot as u8, interest_mask, raw_kind_mask);
    true
}

pub fn tx_set_enabled(state: &mut KernelState, tid: u8, enabled: bool) -> bool {
    let mut i = 0usize;
    while i < state.tx_count as usize {
        if state.tx_used[i] && state.tx_tid[i] == tid {
            state.tx_enabled[i] = enabled;
            return true;
        }
        i += 1;
    }
    false
}

pub fn tx_init_defaults(state: &mut KernelState) {
    if state.tx_count != 0 {
        return;
    }
    let _ = tx_register(
        state,
        0,
        1,
        raw_kind_bit(RAW_INTERNAL) | raw_kind_bit(RAW_IRQ),
        EVT_TIMER,
        tx_ingest_timer,
    );
    let _ = tx_register(
        state,
        1,
        2,
        raw_kind_bit(RAW_SOFTIRQ) | raw_kind_bit(RAW_IRQ) | raw_kind_bit(RAW_MMIO),
        EVT_FAB,
        tx_ingest_fabric,
    );
    let _ = tx_register(
        state,
        2,
        3,
        raw_kind_bit(RAW_SOFTIRQ) | raw_kind_bit(RAW_IRQ) | raw_kind_bit(RAW_MMIO),
        EVT_BUS,
        tx_ingest_bus,
    );
    let _ = tx_register(
        state,
        3,
        4,
        raw_kind_bit(RAW_INTERNAL) | raw_kind_bit(RAW_SOFTIRQ),
        EVT_LOG,
        tx_ingest_log,
    );
}

pub fn tx_step(state: &mut KernelState, raw: RawEvent) -> Option<(u8, VectorEvent)> {
    let rb = raw_kind_bit(raw.kind);
    if rb == 0 {
        return None;
    }
    let mut i = 0usize;
    while i < state.tx_count as usize {
        if state.tx_used[i] && state.tx_enabled[i] && (state.tx_raw_kind_mask[i] & rb) != 0 {
            if let Some(f) = state.tx_ingest[i] {
                if let Some(out) = f(state, raw) {
                    return Some((state.tx_tid[i], out));
                }
            }
        }
        i += 1;
    }
    None
}

pub fn make_softirq_raw(src: u8, softirq_mask_bit: u32) -> Option<RawEvent> {
    let code = if softirq_mask_bit == softirq::SOFTIRQ_BUS {
        RAWC_SOFTIRQ_BUS
    } else if softirq_mask_bit == softirq::SOFTIRQ_FABRIC {
        RAWC_SOFTIRQ_FABRIC
    } else if softirq_mask_bit == softirq::SOFTIRQ_LOG {
        RAWC_SOFTIRQ_LOG
    } else if softirq_mask_bit == softirq::SOFTIRQ_SCHED {
        RAWC_SOFTIRQ_TIMER
    } else {
        return None;
    };
    Some(RawEvent {
        src,
        kind: RAW_SOFTIRQ,
        code,
        arg: 0,
    })
}

fn tx_ingest_timer(_state: &mut KernelState, raw: RawEvent) -> Option<VectorEvent> {
    let ok = (raw.kind == RAW_SOFTIRQ && raw.code == RAWC_SOFTIRQ_TIMER)
        || (raw.kind == RAW_IRQ && raw.code == RAWC_IRQ_GRP_TIMER);
    if !ok {
        return None;
    }
    let mut payload = pack_payload(PTYPE_TIMER, TIO_TIMER, 0, 1, 0);
    payload = payload_set_chan(payload, TIO_TIMER);
    Some(VectorEvent {
        evt_mask: EVT_TIMER,
        src_id: raw.src,
        payload,
    })
}

fn tx_ingest_fabric(_state: &mut KernelState, raw: RawEvent) -> Option<VectorEvent> {
    let (chan, sev) = if raw.kind == RAW_MMIO && raw.code == RAWC_MMIO_FAB_ALERT {
        (TIO_FAB_ALERT, 4)
    } else if (raw.kind == RAW_SOFTIRQ && raw.code == RAWC_SOFTIRQ_FABRIC)
        || (raw.kind == RAW_IRQ && raw.code == RAWC_IRQ_GRP_FAB)
    {
        (TIO_FAB_CTRL, 1)
    } else {
        return None;
    };
    let mut payload = pack_payload(
        PTYPE_FAB,
        chan,
        (raw.arg & 0xFF) as u8,
        sev,
        (raw.arg >> 28) as u8,
    );
    payload = payload_set_chan(payload, chan);
    Some(VectorEvent {
        evt_mask: EVT_FAB,
        src_id: raw.src,
        payload,
    })
}

fn tx_ingest_bus(_state: &mut KernelState, raw: RawEvent) -> Option<VectorEvent> {
    let chan = if raw.kind == RAW_SOFTIRQ && raw.code == RAWC_SOFTIRQ_BUS {
        TIO_BUS_SYS
    } else if raw.kind == RAW_IRQ && raw.code == RAWC_IRQ_GRP_BUS {
        TIO_BUS_SYS
    } else if raw.kind == RAW_MMIO && raw.code == RAWC_MMIO_BUS_RX {
        TIO_BUS_RX
    } else if raw.kind == RAW_MMIO && raw.code == RAWC_MMIO_BUS_TX {
        TIO_BUS_TX
    } else {
        return None;
    };
    let mut payload = pack_payload(
        PTYPE_BUS,
        chan,
        (raw.arg & 0xFF) as u8,
        1,
        (raw.arg >> 28) as u8,
    );
    payload = payload_set_chan(payload, chan);
    Some(VectorEvent {
        evt_mask: EVT_BUS,
        src_id: raw.src,
        payload,
    })
}

fn tx_ingest_log(_state: &mut KernelState, raw: RawEvent) -> Option<VectorEvent> {
    let (chan, sev) = if raw.kind == RAW_SOFTIRQ && raw.code == RAWC_SOFTIRQ_LOG {
        (TIO_LOG_TELEM, 1)
    } else if raw.kind == RAW_INTERNAL && raw.code == RAWC_INTERNAL_LOG_FLUSH {
        (TIO_LOG_TELEM, 1)
    } else if raw.kind == RAW_INTERNAL && raw.code == RAWC_INTERNAL_PANIC {
        (TIO_LOG_PANIC, 4)
    } else {
        return None;
    };
    let mut payload = pack_payload(
        PTYPE_LOG,
        chan,
        (raw.arg & 0xFF) as u8,
        sev,
        (raw.arg >> 28) as u8,
    );
    payload = payload_set_chan(payload, chan);
    Some(VectorEvent {
        evt_mask: EVT_LOG,
        src_id: raw.src,
        payload,
    })
}

fn log_tx_reg(tid: u8, slot: u8, interest: u32, rawk: u32) {
    let mut line = [0u8; 80];
    let mut n = 0usize;
    n += append_bytes2(&mut line[n..], b"T10 tx_reg tid=");
    n += append_u642(&mut line[n..], tid as u64);
    n += append_bytes2(&mut line[n..], b" slot=");
    n += append_u642(&mut line[n..], slot as u64);
    n += append_bytes2(&mut line[n..], b" im=");
    n += append_u642(&mut line[n..], interest as u64);
    n += append_bytes2(&mut line[n..], b" rk=");
    n += append_u642(&mut line[n..], rawk as u64);
    if let Ok(s) = core::str::from_utf8(&line[..n]) {
        log::serial_only(s);
    }
}

fn append_bytes2(dst: &mut [u8], src: &[u8]) -> usize {
    let count = core::cmp::min(dst.len(), src.len());
    dst[..count].copy_from_slice(&src[..count]);
    count
}

fn append_u642(dst: &mut [u8], mut value: u64) -> usize {
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
