use crate::kernel_state::{
    CRIT_DISPATCH_BUDGET, CRIT_DISPATCH_MAX_SLOTS_PER_TICK, EventFrame, KernelState, TaskId,
    TaskState, DL_LOG_MIN_PERIOD_TICKS, DL_LOG_THROTTLE_TICKS, DL_LOG_WINDOW_TICKS, DL_TABLE,
    EC_COUNT, EC_CRIT, EC_HIGH, EC_LOW, EC_NORM, EVT_ALL, EVT_BUS, EVT_FAB, EVT_LOG, EVT_SLOTS,
    EVT_TIMER, EVQ_CAP_CLASS, EVQ_DRAIN_BUDGET, EVQ_DRAIN_QUOTA, EVQ_STARVE_WARN_TICKS, RawEvent,
    RAW_INTERNAL, RAW_SOFTIRQ, RAWC_INTERNAL_LOG_FLUSH, SLOT_BUS, SLOT_FAB,
    SLOT_LOG, SLOT_TIMER, SRC_SOFTIRQ, SRC_TASK, SLEEP_NONE, TASK_DISPATCH_BUDGET,
    TASK_DISPATCH_BUDGET_MAX, TASK_DISPATCH_BUDGET_MIN, TASK_DISPATCH_MAX_SLOTS_PER_TICK,
};
use crate::hal;
use crate::log;
use crate::softirq;
use crate::transjector::{
    bus_step_gated, make_softirq_raw, payload_get_chan, tx_init_defaults, tx_set_enabled, tx_step,
};
use crate::transvector::fabric_step;

pub fn run(state: &mut KernelState) -> ! {
    tx_init_defaults(state);
    log::serial_only("R00 runtime start");
    let _ = state.bootinfo.magic;
    let _ = &mut state.fb;
    let mut last_logged = 0u64;
    loop {
        unsafe {
            core::arch::asm!("hlt");
        }
        let dt = runtime_step(state);
        if (state.tick & 0xFF) == 0 && state.tick != 0 && state.tick != last_logged {
            log_tick(state, dt);
            last_logged = state.tick;
        }
        if (state.tick & 0x1FF) == 0 && state.tick != 0 {
            log_sched(state);
        }
    }
}

fn runtime_step(state: &mut KernelState) -> u64 {
    let irq_tick = hal::time_now_tick();
    let dt = timer_contract_step(state, irq_tick);

    let pending = softirq::take_pending();
    if pending != 0 {
        if (pending & softirq::SOFTIRQ_BUS) != 0 {
            if let Some(raw) = make_softirq_raw(SRC_SOFTIRQ, softirq::SOFTIRQ_BUS) {
                tx_publish_raw(state, raw);
            }
        }
        if (pending & softirq::SOFTIRQ_FABRIC) != 0 {
            if let Some(raw) = make_softirq_raw(SRC_SOFTIRQ, softirq::SOFTIRQ_FABRIC) {
                tx_publish_raw(state, raw);
            }
        }
        if (pending & softirq::SOFTIRQ_LOG) != 0 {
            if let Some(raw) = make_softirq_raw(SRC_SOFTIRQ, softirq::SOFTIRQ_LOG) {
                tx_publish_raw(state, raw);
            }
        }
        if (pending & softirq::SOFTIRQ_BUS) != 0 {
            let _ = bus_step_gated();
        }
        if (pending & softirq::SOFTIRQ_FABRIC) != 0 {
            let _ = fabric_step();
        }
        if (pending & softirq::SOFTIRQ_LOG) != 0 {
            let _ = state.tick;
        }
    }

    event_router_step(state);
    wakeup_step(state);
    adaptive_budget_step(state);

    scheduler_step(state);
    let cur = state.current_task as usize;
    if cur < state.task_count as usize && state.tasks[cur] == TaskState::Ready {
        task_step(state.current_task, state);
    }
    dt
}

fn timer_contract_step(state: &mut KernelState, irq_tick: u64) -> u64 {
    let dt = if state.last_irq_tick == 0 {
        1
    } else if irq_tick >= state.last_irq_tick {
        irq_tick - state.last_irq_tick
    } else {
        0
    };

    if dt > 1 {
        state.missed = state.missed.wrapping_add(dt - 1);
    }
    if dt > state.max_dt {
        state.max_dt = dt;
    }
    if dt > 50 {
        log_jitter(dt);
    }

    state.last_irq_tick = irq_tick;
    state.tick = irq_tick;
    dt
}

fn log_tick(state: &KernelState, dt: u64) {
    let mut line = [0u8; 96];
    let mut n = 0usize;
    n += append_bytes(&mut line[n..], b"R10 tick=");
    n += append_u64(&mut line[n..], state.tick);
    n += append_bytes(&mut line[n..], b" dt=");
    n += append_u64(&mut line[n..], dt);
    n += append_bytes(&mut line[n..], b" miss=");
    n += append_u64(&mut line[n..], state.missed);
    n += append_bytes(&mut line[n..], b" max=");
    n += append_u64(&mut line[n..], state.max_dt);
    if let Ok(s) = core::str::from_utf8(&line[..n]) {
        log::serial_only(s);
    }
}

fn log_jitter(dt: u64) {
    let mut line = [0u8; 48];
    let mut n = 0usize;
    n += append_bytes(&mut line[n..], b"EJ1 jitter dt=");
    n += append_u64(&mut line[n..], dt);
    if let Ok(s) = core::str::from_utf8(&line[..n]) {
        log::serial_only(s);
    }
}

fn adaptive_budget_step(state: &mut KernelState) {
    let pressure = state.evq_len[EC_CRIT]
        .saturating_add(state.evq_len[EC_HIGH])
        .saturating_add(state.evq_len[EC_NORM])
        .saturating_add(state.evq_len[EC_LOW]);

    let mut b0 = TASK_DISPATCH_BUDGET;
    let mut b1 = TASK_DISPATCH_BUDGET;
    let mut b2 = TASK_DISPATCH_BUDGET;

    if pressure >= 12 {
        b1 = TASK_DISPATCH_BUDGET_MIN;
        b2 = (TASK_DISPATCH_BUDGET + 2).min(TASK_DISPATCH_BUDGET_MAX);
    } else if pressure <= 2 {
        b1 = TASK_DISPATCH_BUDGET_MIN;
        b2 = TASK_DISPATCH_BUDGET;
    }

    if state.lat_last[SLOT_BUS] > DL_TABLE[SLOT_BUS] || state.lat_last[SLOT_LOG] > DL_TABLE[SLOT_LOG] {
        b2 = (b2 + 2).min(TASK_DISPATCH_BUDGET_MAX);
    }
    if state.evq_len[EC_CRIT] > 0 {
        b0 = (b0 + 1).min(TASK_DISPATCH_BUDGET_MAX);
    }

    let changed = state.dispatch_budget[0] != b0
        || state.dispatch_budget[1] != b1
        || state.dispatch_budget[2] != b2;

    state.dispatch_budget[0] = b0;
    state.dispatch_budget[1] = b1;
    state.dispatch_budget[2] = b2;

    let now = state.tick as u32;
    let since = now.wrapping_sub(state.dispatch_budget_last_log);
    if (changed && since >= 16) || since >= 256 {
        state.dispatch_budget_last_log = now;
        log_budget(state, pressure);
    }
}

fn scheduler_step(state: &mut KernelState) {
    if state.task_count == 0 || state.task_count as usize > state.tasks.len() {
        return;
    }

    let count = state.task_count as usize;
    let mut next = state.current_task as usize;
    let mut scanned = 0usize;
    while scanned < count {
        next = (next + 1) % count;
        if state.tasks[next] == TaskState::Ready {
            state.current_task = next as TaskId;
            return;
        }
        scanned += 1;
    }
}

fn task_step(task_id: TaskId, state: &mut KernelState) {
    match task_id {
        0 => log_task(state),
        1 => logic_task(state),
        2 => fabric_task(state),
        _ => {}
    }
}

fn idle_task() {}
fn log_task(state: &mut KernelState) {
    let _ = task_dispatch_crit(state, 0);
    let _ = task_dispatch_mail(state, 0, state.dispatch_budget[0]);
    idle_task();
}

fn logic_task(state: &mut KernelState) {
    let _ = task_dispatch_mail(state, 1, state.dispatch_budget[1]);
    state.logic_counter = state.logic_counter.wrapping_add(1);

    if state.logic_counter == 20 {
        task_set_state(state, 2, TaskState::Ready);
        task_subscribe(state, 0, EVT_TIMER);
        task_subscribe(state, 2, EVT_BUS | EVT_FAB | EVT_LOG);
    }

    // S: CRIT bypass under LOG flood.
    if state.logic_counter == 220 {
        task_wait(state, 2, EVT_FAB);
    }
    if state.logic_counter == 240 {
        let mut i = 0u32;
        while i < 40 {
            publish_event_ex(state, EVT_LOG, SRC_TASK, 3000 + i);
            if i == 0 {
                publish_event_ex(state, EVT_TIMER, SRC_TASK, 4000 + i);
            }
            i += 1;
        }
    }
    if state.logic_counter == 252 {
        task_signal(state, 2, EVT_FAB);
    }

    // T: quota fairness for NORM/LOW under flood.
    if state.logic_counter == 280 {
        task_wait(state, 2, EVT_FAB);
    }
    if state.logic_counter == 300 {
        let mut i = 0u32;
        while i < 24 {
            publish_event_ex(state, EVT_BUS, SRC_TASK, 5000 + i);
            publish_event_ex(state, EVT_LOG, SRC_TASK, 6000 + i);
            i += 1;
        }
    }
    if state.logic_counter == 318 {
        task_signal(state, 2, EVT_FAB);
    }

    // U: split by class for mixed mask in one publish.
    if state.logic_counter == 340 {
        task_wait(state, 2, EVT_FAB);
    }
    // W: route correctness with two subscribers on BUS.
    if state.logic_counter == 341 {
        task_subscribe(state, 1, EVT_BUS);
    }
    if state.logic_counter == 342 {
        publish_event_ex(state, EVT_BUS | EVT_LOG, SRC_TASK, 777);
    }
    if state.logic_counter == 344 {
        task_unsubscribe(state, 1, EVT_BUS);
    }
    if state.logic_counter == 345 {
        task_signal(state, 2, EVT_FAB);
    }

    // X: budgeted BUS dispatch burst.
    if state.logic_counter == 360 {
        let mut i = 0u32;
        while i < 10 {
            publish_event_ex(state, EVT_BUS, SRC_TASK, 8000 + i);
            i += 1;
        }
    }

    // Y: slot priority FAB over LOG under one budget.
    if state.logic_counter == 380 {
        publish_event_ex(state, EVT_FAB, SRC_TASK, 8100);
        let mut i = 0u32;
        while i < 10 {
            publish_event_ex(state, EVT_LOG, SRC_TASK, 8200 + i);
            i += 1;
        }
    }

    // Z: force mailbox overflow visibility on BUS.
    if state.logic_counter == 400 {
        let mut i = 0u16;
        while i < 300 {
            mailbox_push(state, 2, EVT_BUS, SRC_TASK, 9000 + i as u32, state.tick as u32);
            i += 1;
        }
        task_signal(state, 2, EVT_BUS);
    }

    // AA1: softirq BUS -> transjector -> publish.
    if state.logic_counter == 420 {
        softirq::raise(softirq::SOFTIRQ_BUS);
    }
    // AA2: unsupported raw should be ignored.
    if state.logic_counter == 422 {
        let raw = RawEvent {
            src: SRC_SOFTIRQ,
            kind: RAW_SOFTIRQ,
            code: 0x01FF,
            arg: 0,
        };
        let _ = tx_step(state, raw);
    }
    // AA3: disable BUS transjector and ensure BUS raw is dropped.
    if state.logic_counter == 424 {
        let _ = tx_set_enabled(state, 3, false);
        softirq::raise(softirq::SOFTIRQ_BUS);
    }
    if state.logic_counter == 426 {
        let _ = tx_set_enabled(state, 3, true);
        softirq::raise(softirq::SOFTIRQ_BUS);
    }
    // AB1: BUS ports via MMIO RX/TX.
    if state.logic_counter == 427 {
        tx_publish_raw(
            state,
            RawEvent {
                src: SRC_SOFTIRQ,
                kind: crate::kernel_state::RAW_MMIO,
                code: crate::kernel_state::RAWC_MMIO_BUS_RX,
                arg: 0,
            },
        );
    }
    if state.logic_counter == 428 {
        tx_publish_raw(
            state,
            RawEvent {
                src: SRC_SOFTIRQ,
                kind: crate::kernel_state::RAW_MMIO,
                code: crate::kernel_state::RAWC_MMIO_BUS_TX,
                arg: 0,
            },
        );
    }
    // AB2: FAB ctrl vs alert.
    if state.logic_counter == 429 {
        tx_publish_raw(
            state,
            RawEvent {
                src: SRC_SOFTIRQ,
                kind: RAW_SOFTIRQ,
                code: crate::kernel_state::RAWC_SOFTIRQ_FABRIC,
                arg: 0,
            },
        );
    }
    if state.logic_counter == 430 {
        tx_publish_raw(
            state,
            RawEvent {
                src: SRC_SOFTIRQ,
                kind: crate::kernel_state::RAW_MMIO,
                code: crate::kernel_state::RAWC_MMIO_FAB_ALERT,
                arg: 0,
            },
        );
    }
    // AB3: LOG telem vs panic.
    if state.logic_counter == 431 {
        tx_publish_raw(
            state,
            RawEvent {
                src: SRC_SOFTIRQ,
                kind: RAW_SOFTIRQ,
                code: crate::kernel_state::RAWC_SOFTIRQ_LOG,
                arg: 0,
            },
        );
    }
    if state.logic_counter == 432 {
        tx_publish_raw(
            state,
            RawEvent {
                src: SRC_SOFTIRQ,
                kind: RAW_INTERNAL,
                code: crate::kernel_state::RAWC_INTERNAL_PANIC,
                arg: 0,
            },
        );
    }
    if state.logic_counter == 433 {
        tx_publish_raw(
            state,
            RawEvent {
                src: SRC_SOFTIRQ,
                kind: RAW_INTERNAL,
                code: RAWC_INTERNAL_LOG_FLUSH,
                arg: 0,
            },
        );
    }

    if state.logic_counter == 4096 {
        task_sleep(state, 1, 200);
    }
    if (state.logic_counter & 0x1FF) == 0 {
        log::serial_only("T01 logic alive");
    }
}

fn fabric_task(state: &mut KernelState) {
    let _ = task_dispatch_mail(state, 2, state.dispatch_budget[2]);
    let wake = task_take_wake(state, 2);
    if wake != 0 {
        log_take_wake(2, wake);
        let again = task_take_wake(state, 2);
        log_take_wake(2, again);
    }
    state.task2_counter = state.task2_counter.wrapping_add(1);
    if (state.task2_counter & 0x3FF) == 0 {
        log::serial_only("T02 alive");
    }
}

pub fn task_set_state(state: &mut KernelState, id: TaskId, new: TaskState) {
    if id >= state.task_count {
        return;
    }
    let idx = id as usize;
    let old = state.tasks[idx];
    if old == new {
        return;
    }
    state.tasks[idx] = new;
    log_task_state_change(id, new);
}

pub fn task_sleep(state: &mut KernelState, id: TaskId, ticks: u64) {
    if id >= state.task_count || ticks == 0 {
        return;
    }
    let idx = id as usize;
    let until = state.tick.wrapping_add(ticks);
    state.wait_mask[idx] = 0;
    state.sleep_until[idx] = until;
    task_set_state(state, id, TaskState::Blocked);
    log_sleep(id, until);
}

pub fn task_wait(state: &mut KernelState, id: TaskId, mask: u32) {
    if id >= state.task_count || mask == 0 {
        return;
    }

    let idx = id as usize;
    log_wait(id, mask);
    state.sleep_until[idx] = SLEEP_NONE;
    state.wait_mask[idx] = mask;
    let matched = state.pending_evt[idx] & mask;
    if matched != 0 {
        wake_task_by_match(state, id, matched, true);
        return;
    }

    task_set_state(state, id, TaskState::Blocked);
}

#[allow(dead_code)]
pub fn task_wait_any(state: &mut KernelState, id: TaskId) {
    task_wait(state, id, EVT_ALL);
}

pub fn task_subscribe(state: &mut KernelState, id: TaskId, mask: u32) {
    if id >= state.task_count || mask == 0 {
        return;
    }
    let idx = id as usize;
    state.sub_mask[idx] |= mask;
    log_sub(id, mask, state.sub_mask[idx]);
}

#[allow(dead_code)]
pub fn task_unsubscribe(state: &mut KernelState, id: TaskId, mask: u32) {
    if id >= state.task_count || mask == 0 {
        return;
    }
    let idx = id as usize;
    state.sub_mask[idx] &= !mask;
    log_unsub(id, mask, state.sub_mask[idx]);
}

#[allow(dead_code)]
pub fn publish_event(state: &mut KernelState, evt: u32) {
    publish_event_ex(state, evt, SRC_SOFTIRQ, 0);
}

pub fn publish_event_ex(state: &mut KernelState, evt_mask: u32, src_id: u8, payload_u32: u32) {
    if evt_mask == 0 {
        return;
    }
    log_pub_ex(evt_mask, src_id, payload_u32);
    let crit = evt_mask & EVT_TIMER;
    let high = evt_mask & EVT_FAB;
    let norm = evt_mask & EVT_BUS;
    let low = evt_mask & EVT_LOG;
    if crit != 0 {
        evq_push(state, EC_CRIT, make_frame(state, crit, src_id, payload_u32));
    }
    if high != 0 {
        evq_push(state, EC_HIGH, make_frame(state, high, src_id, payload_u32));
    }
    if norm != 0 {
        evq_push(state, EC_NORM, make_frame(state, norm, src_id, payload_u32));
    }
    if low != 0 {
        evq_push(state, EC_LOW, make_frame(state, low, src_id, payload_u32));
    }
}

pub fn task_signal(state: &mut KernelState, id: TaskId, mask: u32) {
    if id >= state.task_count || mask == 0 {
        return;
    }

    let idx = id as usize;
    state.pending_evt[idx] |= mask;
    log_signal(id, mask, state.pending_evt[idx]);
    if state.tasks[idx] == TaskState::Blocked
        && state.wait_mask[idx] != 0
        && (state.pending_evt[idx] & state.wait_mask[idx]) != 0
    {
        let matched = state.pending_evt[idx] & state.wait_mask[idx];
        wake_task_by_match(state, id, matched, false);
    }
}

#[allow(dead_code)]
pub fn task_poll(state: &KernelState, id: TaskId, mask: u32) -> u32 {
    if id >= state.task_count || mask == 0 {
        return 0;
    }
    state.pending_evt[id as usize] & mask
}

pub fn task_take_wake(state: &mut KernelState, id: TaskId) -> u32 {
    if id >= state.task_count {
        return 0;
    }
    let idx = id as usize;
    let m = state.last_wake[idx];
    state.last_wake[idx] = 0;
    m
}

pub fn task_take_mail_ex(
    state: &mut KernelState,
    id: TaskId,
) -> (
    u32,
    [u8; EVT_SLOTS],
    u32,
    [u8; EVT_SLOTS],
    [u32; EVT_SLOTS],
    [u32; EVT_SLOTS],
) {
    if id >= state.task_count {
        return (
            0,
            [0; EVT_SLOTS],
            0,
            [0; EVT_SLOTS],
            [0; EVT_SLOTS],
            [0; EVT_SLOTS],
        );
    }
    let idx = id as usize;
    let mask = state.mb_evt[idx];
    let counts = state.mb_cnt[idx];
    let ovf = state.ovf[idx];
    let src = state.mb_src[idx];
    let payload = state.mb_payload[idx];
    let tick = state.mb_tick[idx];
    state.mb_evt[idx] = 0;
    state.mb_cnt[idx] = [0; EVT_SLOTS];
    state.ovf[idx] = 0;
    state.mb_src[idx] = [0; EVT_SLOTS];
    state.mb_payload[idx] = [0; EVT_SLOTS];
    state.mb_tick[idx] = [0; EVT_SLOTS];
    (mask, counts, ovf, src, payload, tick)
}

#[allow(dead_code)]
pub fn task_take_mail(state: &mut KernelState, id: TaskId) -> (u32, [u8; EVT_SLOTS], u32) {
    let (m, c, o, _, _, _) = task_take_mail_ex(state, id);
    (m, c, o)
}

fn task_dispatch_crit(state: &mut KernelState, id: TaskId) -> u8 {
    if id >= state.task_count {
        return 0;
    }
    let idx = id as usize;
    if state.mb_cnt[idx][SLOT_TIMER] == 0 {
        return 0;
    }

    let mut slots_used = 0u8;
    let mut done = 0u8;
    while slots_used < CRIT_DISPATCH_MAX_SLOTS_PER_TICK && done < CRIT_DISPATCH_BUDGET {
        let cnt = state.mb_cnt[idx][SLOT_TIMER];
        if cnt == 0 {
            break;
        }
        let take = core::cmp::min(cnt, CRIT_DISPATCH_BUDGET - done);
        state.mb_cnt[idx][SLOT_TIMER] -= take;
        done += take;
        slots_used += 1;

        state.handled_cnt[idx][SLOT_TIMER] =
            state.handled_cnt[idx][SLOT_TIMER].saturating_add(take as u16);
        let lat = slot_latency(state.tick, state.mb_tick[idx][SLOT_TIMER]);
        state.lat_last[SLOT_TIMER] = lat;
        if lat > state.lat_max[SLOT_TIMER] {
            state.lat_max[SLOT_TIMER] = lat;
        }
        maybe_log_deadline_violation(state, id, SLOT_TIMER, lat);
        log_crit_dispatch(id, take, lat);

        if state.mb_cnt[idx][SLOT_TIMER] == 0 {
            state.mb_evt[idx] &= !EVT_TIMER;
            state.mb_src[idx][SLOT_TIMER] = 0;
            state.mb_payload[idx][SLOT_TIMER] = 0;
            state.mb_tick[idx][SLOT_TIMER] = 0;
        }
    }
    done
}

fn task_dispatch_mail(state: &mut KernelState, id: TaskId, budget: u8) -> u8 {
    if id >= state.task_count || budget == 0 {
        return 0;
    }

    let idx = id as usize;
    let (mask, mut cnt, ovf_mask, src, payload, tick) = task_take_mail_ex(state, id);
    if mask == 0 {
        return 0;
    }

    let order = [SLOT_TIMER, SLOT_FAB, SLOT_BUS, SLOT_LOG];
    let mut budget_left = budget;
    let mut slots_used = 0u8;
    let mut done_total = 0u8;
    let mut done_bus = 0u8;
    let mut done_fab = 0u8;
    let mut done_log = 0u8;
    let mut lat_timer = 0u16;
    let mut lat_bus = 0u16;
    let mut lat_fab = 0u16;
    let mut lat_log = 0u16;

    let mut oi = 0usize;
    while oi < order.len()
        && budget_left > 0
        && slots_used < TASK_DISPATCH_MAX_SLOTS_PER_TICK
    {
        let slot = order[oi];
        let bit = slot_to_evt(slot);
        if (mask & bit) != 0 && cnt[slot] != 0 {
            let take = core::cmp::min(cnt[slot], budget_left);
            cnt[slot] -= take;
            budget_left -= take;
            done_total += take;
            slots_used += 1;

            state.handled_cnt[idx][slot] = state.handled_cnt[idx][slot].saturating_add(take as u16);
            let lat = slot_latency(state.tick, tick[slot]);
            state.lat_last[slot] = lat;
            if lat > state.lat_max[slot] {
                state.lat_max[slot] = lat;
            }
            maybe_log_deadline_violation(state, id, slot, lat);
            if slot == SLOT_BUS {
                done_bus = done_bus.saturating_add(take);
                lat_bus = lat;
            } else if slot == SLOT_FAB {
                done_fab = done_fab.saturating_add(take);
                lat_fab = lat;
            } else if slot == SLOT_LOG {
                done_log = done_log.saturating_add(take);
                lat_log = lat;
            } else if slot == SLOT_TIMER {
                lat_timer = lat;
            }
        }
        oi += 1;
    }

    let mut rem_mask = 0u32;
    let mut si = 0usize;
    while si < EVT_SLOTS {
        if cnt[si] != 0 {
            rem_mask |= slot_to_evt(si);
        }
        si += 1;
    }
    if rem_mask != 0 {
        state.mb_evt[idx] = rem_mask;
        state.mb_cnt[idx] = cnt;
        state.mb_src[idx] = src;
        state.mb_payload[idx] = payload;
        state.mb_tick[idx] = tick;
    }

    if ovf_mask != 0 {
        state.handled_ovf[idx] = state.handled_ovf[idx].saturating_add(1);
        log_task_ovf_seen(id, ovf_mask);
    }

    if done_total > 0 {
        log_dispatch(
            id, mask, done_total, done_bus, done_fab, done_log, lat_timer, lat_bus, lat_fab,
            lat_log,
        );
    }
    done_total
}

fn slot_to_evt(slot: usize) -> u32 {
    if slot == SLOT_TIMER {
        EVT_TIMER
    } else if slot == SLOT_FAB {
        EVT_FAB
    } else if slot == SLOT_BUS {
        EVT_BUS
    } else {
        EVT_LOG
    }
}

fn slot_deadline(slot: usize) -> u16 {
    DL_TABLE[slot]
}

fn slot_latency(now_tick: u64, mb_tick: u32) -> u16 {
    let lat = if now_tick >= mb_tick as u64 {
        now_tick - mb_tick as u64
    } else {
        0
    };
    if lat > u16::MAX as u64 {
        u16::MAX
    } else {
        lat as u16
    }
}

fn maybe_log_deadline_violation(state: &mut KernelState, id: TaskId, slot: usize, lat: u16) {
    let dl = slot_deadline(slot);
    if lat <= dl {
        return;
    }
    state.lat_viol[slot] = state.lat_viol[slot].saturating_add(1);

    let now = state.tick as u32;
    if slot == SLOT_TIMER || slot == SLOT_FAB {
        let last = state.dl_last_log_tick[slot];
        if now.wrapping_sub(last) >= DL_LOG_THROTTLE_TICKS {
            state.dl_last_log_tick[slot] = now;
            log_deadline_violation(slot, lat, dl, id);
        }
        return;
    }

    if state.dl_win_start_tick[slot] == 0 {
        state.dl_win_start_tick[slot] = now;
    }
    state.dl_win_viol[slot] = state.dl_win_viol[slot].saturating_add(1);
    if lat > state.dl_win_max[slot] {
        state.dl_win_max[slot] = lat;
    }

    if state.dl_win_viol[slot] == 1 {
        let last = state.dl_last_log_tick[slot];
        if now.wrapping_sub(last) >= DL_LOG_MIN_PERIOD_TICKS {
            state.dl_last_log_tick[slot] = now;
            log_deadline_violation(slot, lat, dl, id);
        }
    }

    let t0 = state.dl_win_start_tick[slot];
    if now.wrapping_sub(t0) >= DL_LOG_WINDOW_TICKS {
        if state.dl_win_viol[slot] > 0 {
            log_deadline_summary(
                slot,
                state.dl_win_viol[slot],
                state.dl_win_max[slot],
                dl,
                t0,
                now,
            );
        }
        state.dl_win_viol[slot] = 0;
        state.dl_win_max[slot] = 0;
        state.dl_win_start_tick[slot] = now;
    }
}

fn make_frame(state: &KernelState, evt_mask: u32, src_id: u8, payload: u32) -> EventFrame {
    EventFrame {
        evt_mask,
        src_id,
        tick: state.tick as u32,
        payload,
    }
}

fn evq_push(state: &mut KernelState, class: usize, frame: EventFrame) {
    if class >= EC_COUNT {
        return;
    }
    if state.evq_len[class] as usize >= EVQ_CAP_CLASS {
        state.evq_ovf[class] = state.evq_ovf[class].wrapping_add(1);
        log_evq_overflow(class as u8, state.evq_ovf[class]);
        return;
    }
    let tail = state.evq_tail[class] as usize;
    state.evq[class][tail] = frame;
    state.evq_tail[class] = ((tail + 1) % EVQ_CAP_CLASS) as u16;
    state.evq_len[class] += 1;
    log_evq_push(
        class as u8,
        frame.evt_mask,
        frame.src_id,
        frame.payload,
        frame.tick,
        state.evq_len[class],
    );
}

fn evq_pop(state: &mut KernelState, class: usize) -> Option<EventFrame> {
    if class >= EC_COUNT || state.evq_len[class] == 0 {
        return None;
    }
    let head = state.evq_head[class] as usize;
    let frame = state.evq[class][head];
    state.evq_head[class] = ((head + 1) % EVQ_CAP_CLASS) as u16;
    state.evq_len[class] -= 1;
    Some(frame)
}

fn event_router_step(state: &mut KernelState) {
    let mut budget = EVQ_DRAIN_BUDGET;
    let mut drained = [0u16; EC_COUNT];

    let mut class = 0usize;
    while class < EC_COUNT {
        let mut q = 0u16;
        while q < EVQ_DRAIN_QUOTA[class] && budget > 0 {
            let Some(frame) = evq_pop(state, class) else {
                break;
            };
            process_frame(state, class, frame);
            drained[class] += 1;
            budget -= 1;
            q += 1;
        }
        class += 1;
    }

    while budget > 0 {
        let mut picked = false;
        let mut c = 0usize;
        while c < EC_COUNT {
            if let Some(frame) = evq_pop(state, c) {
                process_frame(state, c, frame);
                drained[c] += 1;
                budget -= 1;
                picked = true;
                break;
            }
            c += 1;
        }
        if !picked {
            break;
        }
    }

    update_starvation(state, &drained);
}

fn process_frame(state: &mut KernelState, class: usize, frame: EventFrame) {
    log_evq_pop(
        class as u8,
        frame.evt_mask,
        frame.src_id,
        frame.payload,
        frame.tick,
        state.evq_len[class],
    );

    let count = core::cmp::min(state.task_count as usize, state.tasks.len());
    let mut i = 0usize;
    let mut route_logged = false;
    while i < count {
        let deliver = frame.evt_mask & state.sub_mask[i];
        if deliver != 0 {
            mailbox_push(
                state,
                i as TaskId,
                deliver,
                frame.src_id,
                frame.payload,
                frame.tick,
            );
            task_signal(state, i as TaskId, deliver);
            if !route_logged {
                log_route(class as u8, i as TaskId, deliver, frame.src_id, frame.payload);
                route_logged = true;
            }
        }
        i += 1;
    }
}

fn update_starvation(state: &mut KernelState, drained: &[u16; EC_COUNT]) {
    let mut class = 0usize;
    while class < EC_COUNT {
        if state.evq_len[class] > 0 && drained[class] == 0 {
            state.evq_starve[class] = state.evq_starve[class].saturating_add(1);
            let t = state.evq_starve[class];
            if t >= EVQ_STARVE_WARN_TICKS && (t % EVQ_STARVE_WARN_TICKS) == 0 {
                log_evq_starve(class as u8, t, state.evq_len[class]);
            }
        } else {
            state.evq_starve[class] = 0;
        }
        class += 1;
    }
}

fn mailbox_push(
    state: &mut KernelState,
    id: TaskId,
    deliver_mask: u32,
    src: u8,
    payload: u32,
    tick: u32,
) {
    let idx = id as usize;
    state.mb_evt[idx] |= deliver_mask;

    let (bus, bsrc, bpay) = add_mail_slot(
        state, idx, deliver_mask, EVT_BUS, SLOT_BUS, src, payload, tick,
    );
    let (fab, fsrc, fpay) = add_mail_slot(
        state, idx, deliver_mask, EVT_FAB, SLOT_FAB, src, payload, tick,
    );
    let (logc, lsrc, lpay) = add_mail_slot(
        state, idx, deliver_mask, EVT_LOG, SLOT_LOG, src, payload, tick,
    );
    let _ = add_mail_slot(
        state, idx, deliver_mask, EVT_TIMER, SLOT_TIMER, src, payload, tick,
    );

    log_mailbox(id, deliver_mask, bus, fab, logc, bsrc, bpay, fsrc, fpay, lsrc, lpay);
}

fn add_mail_slot(
    state: &mut KernelState,
    idx: usize,
    deliver_mask: u32,
    evt: u32,
    slot: usize,
    src: u8,
    payload: u32,
    tick: u32,
) -> (u8, u8, u32) {
    if (deliver_mask & evt) == 0 {
        return (state.mb_cnt[idx][slot], state.mb_src[idx][slot], state.mb_payload[idx][slot]);
    }
    let add = 1u16;
    let before = state.mb_cnt[idx][slot];
    let remain = 255u16.saturating_sub(before as u16);
    let applied = core::cmp::min(remain, add);
    state.mb_cnt[idx][slot] = (before as u16 + applied) as u8;
    if add > remain {
        if (state.ovf[idx] & evt) == 0 {
            log_overflow(idx as TaskId, evt);
        }
        state.ovf[idx] |= evt;
    }
    state.mb_src[idx][slot] = src;
    state.mb_payload[idx][slot] = payload;
    state.mb_tick[idx][slot] = tick;
    (state.mb_cnt[idx][slot], state.mb_src[idx][slot], state.mb_payload[idx][slot])
}

fn wakeup_step(state: &mut KernelState) {
    let count = core::cmp::min(state.task_count as usize, state.tasks.len());
    let mut i = 0usize;
    while i < count {
        if state.tasks[i] == TaskState::Blocked {
            if state.wait_mask[i] != 0 {
                let matched = state.pending_evt[i] & state.wait_mask[i];
                if matched != 0 {
                    wake_task_by_match(state, i as TaskId, matched, false);
                }
            } else {
                let until = state.sleep_until[i];
                if until != SLEEP_NONE && state.tick >= until {
                    state.sleep_until[i] = SLEEP_NONE;
                    task_set_state(state, i as TaskId, TaskState::Ready);
                    state.woke = state.woke.wrapping_add(1);
                    log_wake(i as TaskId);
                }
            }
        }
        i += 1;
    }
}

fn wake_task_by_match(state: &mut KernelState, id: TaskId, matched: u32, wait_hit: bool) {
    let idx = id as usize;
    state.pending_evt[idx] &= !matched;
    state.wait_mask[idx] = 0;
    state.last_wake[idx] = matched;
    task_set_state(state, id, TaskState::Ready);
    state.woke = state.woke.wrapping_add(1);
    if wait_hit {
        log_wait_hit(id, matched);
    }
    log_wait_wake(id, matched, state.pending_evt[idx]);
}

fn log_sched(state: &KernelState) {
    let mut ready = 0u64;
    let mut i = 0usize;
    let count = core::cmp::min(state.task_count as usize, state.tasks.len());
    while i < count {
        if state.tasks[i] == TaskState::Ready {
            ready += 1;
        }
        i += 1;
    }

    let mut line = [0u8; 48];
    let mut n = 0usize;
    n += append_bytes(&mut line[n..], b"S10 cur=");
    n += append_u64(&mut line[n..], state.current_task as u64);
    n += append_bytes(&mut line[n..], b" ready=");
    n += append_u64(&mut line[n..], ready);
    if let Ok(s) = core::str::from_utf8(&line[..n]) {
        log::serial_only(s);
    }
}

fn log_budget(state: &KernelState, pressure: u16) {
    let mut line = [0u8; 72];
    let mut n = 0usize;
    n += append_bytes(&mut line[n..], b"R20 bud p=");
    n += append_u64(&mut line[n..], pressure as u64);
    n += append_bytes(&mut line[n..], b" b0=");
    n += append_u64(&mut line[n..], state.dispatch_budget[0] as u64);
    n += append_bytes(&mut line[n..], b" b1=");
    n += append_u64(&mut line[n..], state.dispatch_budget[1] as u64);
    n += append_bytes(&mut line[n..], b" b2=");
    n += append_u64(&mut line[n..], state.dispatch_budget[2] as u64);
    if let Ok(s) = core::str::from_utf8(&line[..n]) {
        log::serial_only(s);
    }
}

fn log_task_state_change(id: TaskId, new: TaskState) {
    let mut line = [0u8; 48];
    let mut n = 0usize;
    n += append_bytes(&mut line[n..], b"S20 task=");
    n += append_u64(&mut line[n..], id as u64);
    n += append_bytes(&mut line[n..], b" -> ");
    n += append_bytes(
        &mut line[n..],
        match new {
            TaskState::Ready => b"Ready",
            TaskState::Blocked => b"Blocked",
        },
    );
    if let Ok(s) = core::str::from_utf8(&line[..n]) {
        log::serial_only(s);
    }
}

fn log_dispatch(
    id: TaskId,
    mask: u32,
    done: u8,
    bus_done: u8,
    fab_done: u8,
    log_done: u8,
    lat_timer: u16,
    lat_bus: u16,
    lat_fab: u16,
    lat_log: u16,
) {
    let mut line = [0u8; 144];
    let mut n = 0usize;
    n += append_bytes(&mut line[n..], b"S80 disp id=");
    n += append_u64(&mut line[n..], id as u64);
    n += append_bytes(&mut line[n..], b" m=");
    n += append_u64(&mut line[n..], mask as u64);
    n += append_bytes(&mut line[n..], b" done=");
    n += append_u64(&mut line[n..], done as u64);
    n += append_bytes(&mut line[n..], b" b=");
    n += append_u64(&mut line[n..], bus_done as u64);
    n += append_bytes(&mut line[n..], b" f=");
    n += append_u64(&mut line[n..], fab_done as u64);
    n += append_bytes(&mut line[n..], b" l=");
    n += append_u64(&mut line[n..], log_done as u64);
    n += append_bytes(&mut line[n..], b" lt=");
    n += append_u64(&mut line[n..], lat_timer as u64);
    n += append_bytes(&mut line[n..], b" lb=");
    n += append_u64(&mut line[n..], lat_bus as u64);
    n += append_bytes(&mut line[n..], b" lf=");
    n += append_u64(&mut line[n..], lat_fab as u64);
    n += append_bytes(&mut line[n..], b" ll=");
    n += append_u64(&mut line[n..], lat_log as u64);
    if let Ok(s) = core::str::from_utf8(&line[..n]) {
        log::serial_only(s);
    }
}

fn log_crit_dispatch(id: TaskId, n_done: u8, lat: u16) {
    let mut line = [0u8; 56];
    let mut n = 0usize;
    n += append_bytes(&mut line[n..], b"S81 crit id=");
    n += append_u64(&mut line[n..], id as u64);
    n += append_bytes(&mut line[n..], b" n=");
    n += append_u64(&mut line[n..], n_done as u64);
    n += append_bytes(&mut line[n..], b" lt=");
    n += append_u64(&mut line[n..], lat as u64);
    if let Ok(s) = core::str::from_utf8(&line[..n]) {
        log::serial_only(s);
    }
}

fn log_deadline_violation(slot: usize, lat: u16, dl: u16, id: TaskId) {
    let mut line = [0u8; 72];
    let mut n = 0usize;
    n += append_bytes(&mut line[n..], b"E50 dl_viol slot=");
    n += append_u64(&mut line[n..], slot as u64);
    n += append_bytes(&mut line[n..], b" lat=");
    n += append_u64(&mut line[n..], lat as u64);
    n += append_bytes(&mut line[n..], b" dl=");
    n += append_u64(&mut line[n..], dl as u64);
    n += append_bytes(&mut line[n..], b" id=");
    n += append_u64(&mut line[n..], id as u64);
    if let Ok(s) = core::str::from_utf8(&line[..n]) {
        log::serial_only(s);
    }
}

fn log_deadline_summary(slot: usize, win_viol: u16, win_max: u16, dl: u16, t0: u32, t1: u32) {
    let mut line = [0u8; 96];
    let mut n = 0usize;
    n += append_bytes(&mut line[n..], b"E51 dl_sum slot=");
    n += append_u64(&mut line[n..], slot as u64);
    n += append_bytes(&mut line[n..], b" v=");
    n += append_u64(&mut line[n..], win_viol as u64);
    n += append_bytes(&mut line[n..], b" mx=");
    n += append_u64(&mut line[n..], win_max as u64);
    n += append_bytes(&mut line[n..], b" dl=");
    n += append_u64(&mut line[n..], dl as u64);
    n += append_bytes(&mut line[n..], b" t0=");
    n += append_u64(&mut line[n..], t0 as u64);
    n += append_bytes(&mut line[n..], b" t1=");
    n += append_u64(&mut line[n..], t1 as u64);
    if let Ok(s) = core::str::from_utf8(&line[..n]) {
        log::serial_only(s);
    }
}

fn log_task_ovf_seen(id: TaskId, ovf_mask: u32) {
    let mut line = [0u8; 48];
    let mut n = 0usize;
    n += append_bytes(&mut line[n..], b"E40 task_ovf_seen id=");
    n += append_u64(&mut line[n..], id as u64);
    n += append_bytes(&mut line[n..], b" o=");
    n += append_u64(&mut line[n..], ovf_mask as u64);
    if let Ok(s) = core::str::from_utf8(&line[..n]) {
        log::serial_only(s);
    }
}

fn log_sleep(id: TaskId, until: u64) {
    let mut line = [0u8; 64];
    let mut n = 0usize;
    n += append_bytes(&mut line[n..], b"S30 sleep id=");
    n += append_u64(&mut line[n..], id as u64);
    n += append_bytes(&mut line[n..], b" until=");
    n += append_u64(&mut line[n..], until);
    if let Ok(s) = core::str::from_utf8(&line[..n]) {
        log::serial_only(s);
    }
}

fn log_wake(id: TaskId) {
    let mut line = [0u8; 32];
    let mut n = 0usize;
    n += append_bytes(&mut line[n..], b"S31 wake id=");
    n += append_u64(&mut line[n..], id as u64);
    if let Ok(s) = core::str::from_utf8(&line[..n]) {
        log::serial_only(s);
    }
}

fn log_wait(id: TaskId, mask: u32) {
    let mut line = [0u8; 48];
    let mut n = 0usize;
    n += append_bytes(&mut line[n..], b"S40 wait id=");
    n += append_u64(&mut line[n..], id as u64);
    n += append_bytes(&mut line[n..], b" m=");
    n += append_u64(&mut line[n..], mask as u64);
    if let Ok(s) = core::str::from_utf8(&line[..n]) {
        log::serial_only(s);
    }
}

fn log_sub(id: TaskId, mask: u32, sub_after: u32) {
    let mut line = [0u8; 64];
    let mut n = 0usize;
    n += append_bytes(&mut line[n..], b"S50 sub id=");
    n += append_u64(&mut line[n..], id as u64);
    n += append_bytes(&mut line[n..], b" m=");
    n += append_u64(&mut line[n..], mask as u64);
    n += append_bytes(&mut line[n..], b" s=");
    n += append_u64(&mut line[n..], sub_after as u64);
    if let Ok(s) = core::str::from_utf8(&line[..n]) {
        log::serial_only(s);
    }
}

#[allow(dead_code)]
fn log_unsub(id: TaskId, mask: u32, sub_after: u32) {
    let mut line = [0u8; 68];
    let mut n = 0usize;
    n += append_bytes(&mut line[n..], b"S51 unsub id=");
    n += append_u64(&mut line[n..], id as u64);
    n += append_bytes(&mut line[n..], b" m=");
    n += append_u64(&mut line[n..], mask as u64);
    n += append_bytes(&mut line[n..], b" s=");
    n += append_u64(&mut line[n..], sub_after as u64);
    if let Ok(s) = core::str::from_utf8(&line[..n]) {
        log::serial_only(s);
    }
}

fn log_pub_ex(evt: u32, src: u8, payload: u32) {
    let mut line = [0u8; 72];
    let mut n = 0usize;
    n += append_bytes(&mut line[n..], b"S52 pub e=");
    n += append_u64(&mut line[n..], evt as u64);
    n += append_bytes(&mut line[n..], b" src=");
    n += append_u64(&mut line[n..], src as u64);
    n += append_bytes(&mut line[n..], b" pay=");
    n += append_u64(&mut line[n..], payload as u64);
    if let Ok(s) = core::str::from_utf8(&line[..n]) {
        log::serial_only(s);
    }
}

fn tx_publish_raw(state: &mut KernelState, raw: RawEvent) {
    if let Some((tid, out)) = tx_step(state, raw) {
        publish_event_ex(state, out.evt_mask, out.src_id, out.payload);
        log_tx_out(tid, out.evt_mask, out.src_id, out.payload);
    }
}

fn log_tx_out(tid: u8, evt: u32, src: u8, payload: u32) {
    let mut line = [0u8; 104];
    let mut n = 0usize;
    n += append_bytes(&mut line[n..], b"T11 tx_out tid=");
    n += append_u64(&mut line[n..], tid as u64);
    n += append_bytes(&mut line[n..], b" e=");
    n += append_u64(&mut line[n..], evt as u64);
    n += append_bytes(&mut line[n..], b" src=");
    n += append_u64(&mut line[n..], src as u64);
    n += append_bytes(&mut line[n..], b" p=");
    n += append_u64(&mut line[n..], payload as u64);
    n += append_bytes(&mut line[n..], b" chan=");
    n += append_u64(&mut line[n..], payload_get_chan(payload) as u64);
    if let Ok(s) = core::str::from_utf8(&line[..n]) {
        log::serial_only(s);
    }
}

fn log_evq_push(class: u8, evt: u32, src: u8, payload: u32, tick: u32, len: u16) {
    let mut line = [0u8; 128];
    let mut n = 0usize;
    n += append_bytes(&mut line[n..], b"S70 evq_push c=");
    n += append_u64(&mut line[n..], class as u64);
    n += append_bytes(&mut line[n..], b" e=");
    n += append_u64(&mut line[n..], evt as u64);
    n += append_bytes(&mut line[n..], b" src=");
    n += append_u64(&mut line[n..], src as u64);
    n += append_bytes(&mut line[n..], b" pay=");
    n += append_u64(&mut line[n..], payload as u64);
    n += append_bytes(&mut line[n..], b" t=");
    n += append_u64(&mut line[n..], tick as u64);
    n += append_bytes(&mut line[n..], b" len=");
    n += append_u64(&mut line[n..], len as u64);
    if let Ok(s) = core::str::from_utf8(&line[..n]) {
        log::serial_only(s);
    }
}

fn log_evq_pop(class: u8, evt: u32, src: u8, payload: u32, tick: u32, len_after: u16) {
    let mut line = [0u8; 128];
    let mut n = 0usize;
    n += append_bytes(&mut line[n..], b"S71 evq_pop c=");
    n += append_u64(&mut line[n..], class as u64);
    n += append_bytes(&mut line[n..], b" e=");
    n += append_u64(&mut line[n..], evt as u64);
    n += append_bytes(&mut line[n..], b" src=");
    n += append_u64(&mut line[n..], src as u64);
    n += append_bytes(&mut line[n..], b" pay=");
    n += append_u64(&mut line[n..], payload as u64);
    n += append_bytes(&mut line[n..], b" t=");
    n += append_u64(&mut line[n..], tick as u64);
    n += append_bytes(&mut line[n..], b" len=");
    n += append_u64(&mut line[n..], len_after as u64);
    if let Ok(s) = core::str::from_utf8(&line[..n]) {
        log::serial_only(s);
    }
}

fn log_evq_overflow(class: u8, count: u32) {
    let mut line = [0u8; 48];
    let mut n = 0usize;
    n += append_bytes(&mut line[n..], b"E30 evq_ovf c=");
    n += append_u64(&mut line[n..], class as u64);
    n += append_bytes(&mut line[n..], b" n=");
    n += append_u64(&mut line[n..], count as u64);
    if let Ok(s) = core::str::from_utf8(&line[..n]) {
        log::serial_only(s);
    }
}

fn log_evq_starve(class: u8, ticks: u16, len: u16) {
    let mut line = [0u8; 64];
    let mut n = 0usize;
    n += append_bytes(&mut line[n..], b"E31 starve c=");
    n += append_u64(&mut line[n..], class as u64);
    n += append_bytes(&mut line[n..], b" t=");
    n += append_u64(&mut line[n..], ticks as u64);
    n += append_bytes(&mut line[n..], b" len=");
    n += append_u64(&mut line[n..], len as u64);
    if let Ok(s) = core::str::from_utf8(&line[..n]) {
        log::serial_only(s);
    }
}

fn log_route(class: u8, id: TaskId, deliver: u32, src: u8, payload: u32) {
    let mut line = [0u8; 112];
    let mut n = 0usize;
    n += append_bytes(&mut line[n..], b"S53 route c=");
    n += append_u64(&mut line[n..], class as u64);
    n += append_bytes(&mut line[n..], b" id=");
    n += append_u64(&mut line[n..], id as u64);
    n += append_bytes(&mut line[n..], b" d=");
    n += append_u64(&mut line[n..], deliver as u64);
    n += append_bytes(&mut line[n..], b" src=");
    n += append_u64(&mut line[n..], src as u64);
    n += append_bytes(&mut line[n..], b" pay=");
    n += append_u64(&mut line[n..], payload as u64);
    if let Ok(s) = core::str::from_utf8(&line[..n]) {
        log::serial_only(s);
    }
}

fn log_mailbox(
    id: TaskId,
    deliver_mask: u32,
    bus: u8,
    fab: u8,
    logc: u8,
    bsrc: u8,
    bpay: u32,
    fsrc: u8,
    fpay: u32,
    lsrc: u8,
    lpay: u32,
) {
    let mut line = [0u8; 192];
    let mut n = 0usize;
    n += append_bytes(&mut line[n..], b"S60 mb id=");
    n += append_u64(&mut line[n..], id as u64);
    n += append_bytes(&mut line[n..], b" d=");
    n += append_u64(&mut line[n..], deliver_mask as u64);
    n += append_bytes(&mut line[n..], b" cB=");
    n += append_u64(&mut line[n..], bus as u64);
    n += append_bytes(&mut line[n..], b" cF=");
    n += append_u64(&mut line[n..], fab as u64);
    n += append_bytes(&mut line[n..], b" cL=");
    n += append_u64(&mut line[n..], logc as u64);
    n += append_bytes(&mut line[n..], b" srcB=");
    n += append_u64(&mut line[n..], bsrc as u64);
    n += append_bytes(&mut line[n..], b" payB=");
    n += append_u64(&mut line[n..], bpay as u64);
    n += append_bytes(&mut line[n..], b" srcF=");
    n += append_u64(&mut line[n..], fsrc as u64);
    n += append_bytes(&mut line[n..], b" payF=");
    n += append_u64(&mut line[n..], fpay as u64);
    n += append_bytes(&mut line[n..], b" srcL=");
    n += append_u64(&mut line[n..], lsrc as u64);
    n += append_bytes(&mut line[n..], b" payL=");
    n += append_u64(&mut line[n..], lpay as u64);
    if let Ok(s) = core::str::from_utf8(&line[..n]) {
        log::serial_only(s);
    }
}

fn log_overflow(id: TaskId, evt: u32) {
    let mut line = [0u8; 40];
    let mut n = 0usize;
    n += append_bytes(&mut line[n..], b"E20 ovf id=");
    n += append_u64(&mut line[n..], id as u64);
    n += append_bytes(&mut line[n..], b" e=");
    n += append_u64(&mut line[n..], evt as u64);
    if let Ok(s) = core::str::from_utf8(&line[..n]) {
        log::serial_only(s);
    }
}

#[allow(dead_code)]
fn log_take_mail(
    id: TaskId,
    mask: u32,
    bus: u8,
    fab: u8,
    logc: u8,
    ovf: u32,
    bsrc: u8,
    bpay: u32,
    fsrc: u8,
    fpay: u32,
    lsrc: u8,
    lpay: u32,
) {
    let mut line = [0u8; 208];
    let mut n = 0usize;
    n += append_bytes(&mut line[n..], b"S61 take_mb id=");
    n += append_u64(&mut line[n..], id as u64);
    n += append_bytes(&mut line[n..], b" m=");
    n += append_u64(&mut line[n..], mask as u64);
    n += append_bytes(&mut line[n..], b" b=");
    n += append_u64(&mut line[n..], bus as u64);
    n += append_bytes(&mut line[n..], b" f=");
    n += append_u64(&mut line[n..], fab as u64);
    n += append_bytes(&mut line[n..], b" l=");
    n += append_u64(&mut line[n..], logc as u64);
    n += append_bytes(&mut line[n..], b" o=");
    n += append_u64(&mut line[n..], ovf as u64);
    n += append_bytes(&mut line[n..], b" srcB=");
    n += append_u64(&mut line[n..], bsrc as u64);
    n += append_bytes(&mut line[n..], b" payB=");
    n += append_u64(&mut line[n..], bpay as u64);
    n += append_bytes(&mut line[n..], b" srcF=");
    n += append_u64(&mut line[n..], fsrc as u64);
    n += append_bytes(&mut line[n..], b" payF=");
    n += append_u64(&mut line[n..], fpay as u64);
    n += append_bytes(&mut line[n..], b" srcL=");
    n += append_u64(&mut line[n..], lsrc as u64);
    n += append_bytes(&mut line[n..], b" payL=");
    n += append_u64(&mut line[n..], lpay as u64);
    if let Ok(s) = core::str::from_utf8(&line[..n]) {
        log::serial_only(s);
    }
}

fn log_wait_hit(id: TaskId, mask: u32) {
    let mut line = [0u8; 52];
    let mut n = 0usize;
    n += append_bytes(&mut line[n..], b"S41 wait hit id=");
    n += append_u64(&mut line[n..], id as u64);
    n += append_bytes(&mut line[n..], b" m=");
    n += append_u64(&mut line[n..], mask as u64);
    if let Ok(s) = core::str::from_utf8(&line[..n]) {
        log::serial_only(s);
    }
}

fn log_signal(id: TaskId, evt: u32, pending_after: u32) {
    let mut line = [0u8; 64];
    let mut n = 0usize;
    n += append_bytes(&mut line[n..], b"S41 sig to=");
    n += append_u64(&mut line[n..], id as u64);
    n += append_bytes(&mut line[n..], b" e=");
    n += append_u64(&mut line[n..], evt as u64);
    n += append_bytes(&mut line[n..], b" p=");
    n += append_u64(&mut line[n..], pending_after as u64);
    if let Ok(s) = core::str::from_utf8(&line[..n]) {
        log::serial_only(s);
    }
}

fn log_wait_wake(id: TaskId, matched: u32, pending_after: u32) {
    let mut line = [0u8; 64];
    let mut n = 0usize;
    n += append_bytes(&mut line[n..], b"S42 wake id=");
    n += append_u64(&mut line[n..], id as u64);
    n += append_bytes(&mut line[n..], b" m=");
    n += append_u64(&mut line[n..], matched as u64);
    n += append_bytes(&mut line[n..], b" p=");
    n += append_u64(&mut line[n..], pending_after as u64);
    if let Ok(s) = core::str::from_utf8(&line[..n]) {
        log::serial_only(s);
    }
}

fn log_take_wake(id: TaskId, mask: u32) {
    let mut line = [0u8; 40];
    let mut n = 0usize;
    n += append_bytes(&mut line[n..], b"S43 take id=");
    n += append_u64(&mut line[n..], id as u64);
    n += append_bytes(&mut line[n..], b" m=");
    n += append_u64(&mut line[n..], mask as u64);
    if let Ok(s) = core::str::from_utf8(&line[..n]) {
        log::serial_only(s);
    }
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
