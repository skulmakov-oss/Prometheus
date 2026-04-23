# Baseline Invariants and DoD (v0.x)

Status: active
Updated: 2026-03-03
Scope: Prometheus baseline stabilization before syscall/ABI, WM skeleton, and formal Prometheus <-> EXOcode contract.

## 1. Definition of Done (Baseline Gate)
Baseline is considered PASS only if all checks below are true on a baseline-mode run:

1. Boot path success:
- Markers present in order: `Prometheus Bootloader`, `GOP OK`, `Disk OK`, `Kernel loaded`, `ExitBootServices OK`, `Jump to kernel`.

2. Kernel/runtime start:
- Markers present: `K00`, `K01`, `K07`, `R00 runtime start`.

3. Liveness:
- Periodic markers present after runtime start: `R10`, `S10`, `T01 logic alive`, `T02 alive`.

4. Tick monotonicity:
- `R10 tick=` is strictly increasing.
- `dt >= 1` in normal steady state.
- No negative or wrapped-back tick observations.

5. Event queue and mailbox health:
- No `E30 evq_ovf`.
- No `E20 ovf`.
- No `E31 starve` during baseline campaign.

6. Deadline health:
- No `E50 dl_viol`.
- No `E51 dl_sum`.

7. Fault and panic safety:
- No panic markers (`PXX`/panic text once implemented).
- No exception fatal markers (`XUD/XGP/XPF/XDF` once implemented) in normal baseline run.
- No reboot loop, no silent log cutoff while VM is expected alive.

8. Campaign consistency:
- 10+ consecutive cold boots must each satisfy items 1-7.

## 2. PASS/FAIL Thresholds

### Per single baseline run
PASS if:
- All mandatory boot/runtime markers exist.
- Tick monotonicity check passes.
- Error counters are zero for: `E20`, `E30`, `E31`, `E40`, `E50`, `E51`.
- No panic/fault/reset signature.

FAIL if any is true:
- Missing required marker sequence.
- Tick not monotonic.
- Any `E20/E30/E31/E40/E50/E51` detected.
- Panic/exception/reset signature detected.
- Runtime silence beyond monitoring timeout (suggested: > 5s with no serial lines after runtime start under expected active VM).

### For campaign (10-20 cold boots)
PASS if:
- At least 10 consecutive runs PASS.
- Aggregate error counts for `E20/E30/E31/E40/E50/E51` are all zero.

FAIL if:
- Any run FAILs.
- Non-consecutive pass streak < 10.

## 3. Reset / Silent Fault Criteria
A run is marked `reset_or_silent_fault=true` if any condition holds:

1. Boot markers repeat in same log after runtime started (indicates reboot loop).
2. Runtime started (`R00`) but no liveness (`R10`/`S10`) appears within expected warmup window.
3. Log stream stops unexpectedly without clean QEMU exit evidence and without terminal fatal marker.
4. Known QEMU reset/fault signatures present (parser keyword list should include `triple fault`, `reset`, `CPU Reset` when available in host output).

## 4. Marker Dictionary (Current + Reserved Additions)
Rule: existing markers are immutable. New markers may only be additive.

### Existing boot/runtime markers
- Bootloader textual:
  - `Prometheus Bootloader`
  - `GOP OK`
  - `Disk OK`
  - `Kernel loaded`
  - `ExitBootServices OK`
  - `Jump to kernel`
- Kernel stage:
  - `K00..K07`
- Runtime:
  - `R00 runtime start`
  - `R10 tick=... dt=... miss=... max=...`
  - `S10 cur=... ready=...`
  - `T01 logic alive`
  - `T02 alive`

### Existing diagnostics
- Queue/mailbox:
  - `E20 ovf id=... e=...`
  - `E30 evq_ovf c=... n=...`
  - `E31 starve c=... t=... len=...`
  - `E40 task_ovf_seen id=... o=...`
- Deadline:
  - `E50 dl_viol slot=... lat=... dl=... id=...`
  - `E51 dl_sum slot=... v=... mx=... dl=... t0=... t1=...`
- Timing:
  - `EJ1 jitter dt=...`

### Reserved additive markers for upcoming stages
- Exceptions (stage 1):
  - `XUD` invalid opcode fatal
  - `XGP` general protection fatal
  - `XPF` page fault fatal
  - `XDF` double fault fatal
- Panic (stage 3):
  - `PXX` panic marker family (exact suffix documented when implemented)
- Baseline mode info (stage 4):
  - `B00 baseline mode enabled`

## 5. Operational Modes

### `baseline` mode (target for campaign)
- Stress injections disabled.
- Invariant checks active.
- Liveness markers active.
- Objective: zero `E20/E30/E31/E40/E50/E51` if system is healthy.

### default/stress mode
- Existing stress scenarios remain allowed for diagnostics.
- Error markers may appear by design.
- Not used for baseline PASS certification.

## 6. Non-Functional Constraints (Must Hold)
1. Deterministic bounded runtime behavior.
2. No heap allocation in IRQ/exception/hot path.
3. Preserve quad semantics (`N/F/T/S`) and transvector merge model.
4. No premature ABI/EXOcode contract before baseline gate closure.

## 7. Required Artifacts for Baseline Campaign
- `dist/logs/baseline/boot_001.raw.log` ... `boot_020.raw.log`
- `dist/logs/baseline/summary.json`
- Parser output per run:
  - `report.json`
  - short text verdict (`PASS`/`FAIL` + reason)

## 8. Minimal Parser Contract (for stage 7)
Parser must output at least:

```json
{
  "run_id": "boot_001",
  "pass": true,
  "errors": {
    "E20": 0,
    "E30": 0,
    "E31": 0,
    "E40": 0,
    "E50": 0,
    "E51": 0
  },
  "markers": {
    "boot_sequence_ok": true,
    "runtime_started": true,
    "liveness_ok": true,
    "tick_monotonic": true
  },
  "reset_or_silent_fault": false,
  "notes": []
}
```

## 9. Change Control
- Any modification of thresholds or mandatory markers must update this file in the same commit.
- Existing marker names must not be renamed or removed.

## 10. Exception Acceptance (A1)

Purpose: verify exception handlers (`XUD/XGP/XPF/XDF`) produce stable dumps with `lm` and halt without silent reset.

### Kernel features
- `exc_ud`
- `exc_gp`
- `exc_pf`
- `exc_df`

Only one exception feature must be enabled per run.

### Run commands (examples)

UD:
```powershell
$env:PROMETHEUS_KERNEL_FEATURES = "baseline,exc_ud"
.\run.ps1
```

GP:
```powershell
$env:PROMETHEUS_KERNEL_FEATURES = "baseline,exc_gp"
.\run.ps1
```

PF:
```powershell
$env:PROMETHEUS_KERNEL_FEATURES = "baseline,exc_pf"
.\run.ps1
```

DF (best effort acceptance path):
```powershell
$env:PROMETHEUS_KERNEL_FEATURES = "baseline,exc_df"
.\run.ps1
```

Reset env after test:
```powershell
Remove-Item Env:PROMETHEUS_KERNEL_FEATURES -ErrorAction SilentlyContinue
```

### Parser mode for exception acceptance
Use the Rust parser in `exc_acceptance` mode:
```powershell
cd .\tools\log_parser
cargo run --quiet --target x86_64-pc-windows-msvc -- --input <raw_log_path> --output <report_json_path> --mode exc_acceptance
```

Expected verdict: `FAIL` with note `exception=XUD|XGP|XPF|XDF` and no reset-hint notes.

## Baseline Campaign Evidence

Date: 2026-03-04

Command:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File C:\Users\said3\Desktop\Prometheus\tools\baseline_campaign.ps1 -Boots 10 -MinConsecutivePass 10 -KernelFeatures baseline -RunTimeoutSec 20
```

Result: `10/10 PASS`, `consecutive_pass=10`, `campaign_pass=true`.

Evidence path:

- `C:\Users\said3\Desktop\Prometheus\dist\logs\baseline\summary.json`
- `C:\Users\said3\Desktop\Prometheus\dist\logs\baseline\boot_001.raw.log` ... `boot_010.raw.log`
- `C:\Users\said3\Desktop\Prometheus\dist\logs\baseline\boot_001.report.json` ... `boot_010.report.json`
