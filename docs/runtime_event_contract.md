Runtime Event Contract v1.8

- Event classes:
  - `EC_CRIT=0`, `EC_HIGH=1`, `EC_NORM=2`, `EC_LOW=3`.
  - Mapping: `TIMER->CRIT`, `FAB->HIGH`, `BUS->NORM`, `LOG->LOW`.

- Queue topology:
  - Four bounded FIFO rings, one per class:
  - `evq[EC_COUNT][EVQ_CAP_CLASS]`, `evq_head[]`, `evq_tail[]`, `evq_len[]`.
  - Overflow counter per class: `evq_ovf[class]`.
  - Starvation counter per class: `evq_starve[class]`.

- Publish:
  - `publish_event_ex(mask, src, payload)` splits mixed masks by class.
  - Each class fragment is pushed as its own frame.
  - Push log: `S70 evq_push c=<class> e=<mask> src=<src> pay=<payload> len=<len>`.
  - Overflow log: `E30 evq_ovf c=<class> n=<cnt>`.

- Deterministic drain policy:
  - Per tick budget: `EVQ_DRAIN_BUDGET`.
  - First pass by quotas: `EVQ_DRAIN_QUOTA[class]` for CRIT/HIGH/NORM/LOW.
  - Remaining budget is consumed by priority order CRIT->HIGH->NORM->LOW.
  - Pop log: `S71 evq_pop c=<class> e=<mask> src=<src> pay=<payload> len=<len_after>`.
  - Route trace (v1.8.1): `S53 route c=<class> id=<id> d=<deliver> src=<src> pay=<payload>`.
  - Throttle rule: at most one `S53` is emitted per popped frame (`S71`), using the first delivered subscriber.

- Starvation monitor:
  - If `evq_len[class] > 0` and class drained `0` frames in a tick, `evq_starve[class]++`.
  - When threshold hits (`EVQ_STARVE_WARN_TICKS`), log:
  - `E31 starve c=<class> t=<ticks> len=<len>`.

- Delivery semantics:
  - For each popped frame and task: `deliver = frame.evt_mask & sub_mask[id]`.
  - If non-zero: mailbox update (`mb_evt`, `mb_cnt`, `mb_src`, `mb_payload`) then `task_signal`.
  - Inbox API unchanged: `task_take_mail_ex(id) -> (mask, cnt[4], ovf, src[4], payload[4])`.

- Task dispatch (v1.9):
  - `task_dispatch_mail(state, id, budget)` consumes mailbox with bounded work per tick.
  - Budget constants: `TASK_DISPATCH_BUDGET=4`, `TASK_DISPATCH_MAX_SLOTS_PER_TICK=2`.
  - Slot priority: `TIMER -> FAB -> BUS -> LOG`.
  - Partial consume is preserved by re-queuing remaining mailbox counters for next ticks.
  - Stats: `handled_cnt[id][slot]`, `handled_ovf[id]`.
  - Dispatch log (throttled to calls with work): `S80 disp id=<id> m=<mask> done=<n> b=<bus> f=<fab> l=<log>`.
  - Overflow observed in dispatch: `E40 task_ovf_seen id=<id> o=<ovf_mask>`.

- Latency + deadline watchdog (v1.10):
  - Each mailbox slot keeps last delivery tick: `mb_tick[id][slot]` (last-write-wins).
  - On dispatch, latency is measured: `lat = now_tick - mb_tick`.
  - Runtime tracks:
  - `lat_last[slot]`, `lat_max[slot]`, `lat_viol[slot]`.
  - Deadlines by slot:
  - `TIMER=2`, `FAB=4`, `BUS=8`, `LOG=32` ticks.
  - Violation log (throttled by `DL_LOG_THROTTLE_TICKS`):
  - `E50 dl_viol slot=<s> lat=<lat> dl=<dl> id=<task>`.
  - Dispatch log includes latency snapshot:
  - `S80 ... lt=<timer> lb=<bus> lf=<fab> ll=<log>`.

- CRIT fast-lane dispatch (v1.10.1):
  - Before normal dispatch, service task0 runs `task_dispatch_crit`.
  - Fast-lane handles only `SLOT_TIMER` with bounded budget:
  - `CRIT_DISPATCH_BUDGET=2`, `CRIT_DISPATCH_MAX_SLOTS_PER_TICK=1`.
  - It updates the same latency/deadline counters and clears only TIMER mailbox bits.
  - Optional log on work: `S81 crit id=<id> n=<n> lt=<lat>`.

- Deadline log noise control (v1.10.2):
  - Deadline accounting is unchanged (`lat_viol` still increments on every violation).
  - For `TIMER/FAB` slots: `E50` behavior remains immediate+throttled.
  - For `BUS/LOG` slots:
  - Optional first-hit `E50` (min period: `DL_LOG_MIN_PERIOD_TICKS`),
  - Windowed summary `E51 dl_sum slot=<s> v=<viol> mx=<max> dl=<dl> t0=<start> t1=<now>`
    every `DL_LOG_WINDOW_TICKS`.

- Adaptive budgets (v1.11):
  - Runtime adjusts per-task dispatch budgets each tick from queue pressure/latency.
  - `dispatch_budget[0..2]` is used by service tasks instead of fixed budget.
  - Policy is deterministic and bounded:
  - pressure high (`evq_len sum >= 12`) shifts budget from task1 to task2,
  - BUS/LOG latency over deadline temporarily boosts task2 budget,
  - CRIT queue presence boosts task0 budget by +1.
  - Telemetry log: `R20 bud p=<pressure> b0=<..> b1=<..> b2=<..>` (on change or periodic).

- Transjector Core v0 (v1.12):
  - Raw input contract: `RawEvent { src, kind, code, arg }`.
  - Semantic output contract: `VectorEvent { evt_mask, src_id, payload }`.
  - Runtime path for softirq: `raw -> tx_step -> publish_event_ex`.
  - Fixed registration slots (0..3): timer/fabric/bus/log via `tx_init_defaults`.
  - Logs:
  - `T10 tx_reg ...` on registration,
  - `T11 tx_out ...` when transjector emits a semantic event.
  - Payload pack v0:
  - `ptype(0..7) | chan(8..15) | id(16..23) | sev(24..27) | flags(28..31)`.

- Transjector IO Ports (v1.14):
  - Canonical ports (`chan` field in payload):
  - `TIO_TIMER=0`, `TIO_BUS_SYS=1`, `TIO_BUS_RX=2`, `TIO_BUS_TX=3`,
    `TIO_FAB_CTRL=4`, `TIO_FAB_ALERT=5`, `TIO_LOG_TELEM=6`, `TIO_LOG_PANIC=7`.
  - Mapping examples:
  - `SOFTIRQ_BUS -> EVT_BUS + chan=1`,
  - `MMIO_BUS_RX/TX -> EVT_BUS + chan=2/3`,
  - `SOFTIRQ_FABRIC -> EVT_FAB + chan=4`,
  - `MMIO_FAB_ALERT -> EVT_FAB + chan=5 + sev>=3`,
  - `SOFTIRQ_LOG -> EVT_LOG + chan=6`,
  - `INTERNAL_PANIC -> EVT_LOG + chan=7 + sev=4`.
  - `T11 tx_out` now includes channel:
  - `T11 tx_out tid=<tid> e=<evt> src=<src> p=<payload> chan=<chan>`.

- HAL Contract v0 (v1.13):
  - Minimal hardware abstraction entry points:
  - `hal::time_now_tick()`
  - `hal::irq_ack_timer()`
  - `hal::serial_write_line()`
  - `hal::mmio_read32()/hal::mmio_write32()` (unsafe)
  - `hal::framebuffer_putpixel()` (unsafe)
  - Runtime now reads current tick through HAL (`hal::time_now_tick`) instead of
    directly from platform interrupt backend.
