# VectorOS
<img width="1536" height="1024" alt="ChatGPT Image 9 февр  2026 г , 16_29_39" src="https://github.com/user-attachments/assets/6b5e7107-6f94-40da-93df-cfb75bc4a79a" />

VectorOS is a minimal OS project built in Rust (`no_std`) with a deterministic kernel runtime and a semantic event pipeline.

Current focus:
1. Reliable boot path (UEFI bootloader -> ELF kernel handoff).
2. Deterministic runtime (IRQ/softirq/event routing/dispatch).
3. Transvector/Transjector model as a portability spine for future HAL and multi-platform support.

## Architecture Principles

VectorOS is built around this stack:
1. `HAL`: minimal hardware-facing primitives (timer/irq/serial/framebuffer).
2. `Transjector Layer`: normalize raw platform signals into stable semantic events.
3. `Deterministic Runtime`: route, filter, dispatch, and account events with bounded behavior.

Design constraints:
1. `no_std`, no heap allocators in core runtime paths.
2. Fixed-size buffers and tables.
3. Deterministic ordering and bounded work per tick.
4. Compact diagnostics through framebuffer + serial logs.

## Boot Flow

1. UEFI bootloader (`x86_64-unknown-uefi`) starts.
2. Bootloader initializes UEFI services and GOP framebuffer.
3. Bootloader loads `\EFI\BOOT\KERNEL.ELF` from FAT.
4. ELF `PT_LOAD` segments are mapped/copied, BSS is zeroed.
5. Bootloader gets memory map, calls `ExitBootServices`.
6. Control is transferred to kernel entry:
   `extern "C" fn kernel_main(info: *const BootInfo) -> !`

Bootloader expected screen log:

```text
VectorOS Bootloader
GOP OK
Disk OK
Kernel loaded
ExitBootServices OK
Jump to kernel
```

## BootInfo ABI

Bootloader and kernel share a strict ABI contract:

```rust
#[repr(C)]
pub struct BootInfo {
    pub magic: u64,
    pub framebuffer: Framebuffer,
    pub memory_map: MemoryMapInfo,
    pub rsdp_addr: u64,
}

#[repr(C)]
pub struct Framebuffer {
    pub base: u64,
    pub size: u64,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub format: u32,
}

#[repr(C)]
pub struct MemoryMapInfo {
    pub map: *const u8,
    pub size: usize,
    pub desc_size: usize,
}
```

`magic` value: `0x534F544345565F56`.

## Kernel Runtime Overview

Kernel side currently includes:
1. BootInfo validation guards (`E01..E06` style checks).
2. Framebuffer text output + serial diagnostics.
3. Runtime loop with tick accounting and jitter/deadline monitoring.
4. Event transport pipeline:
   `IRQ/softirq -> Event Queue -> Router -> Mailbox -> Task Dispatch`.
5. Task model (deterministic scheduler skeleton, task states, wake/signal semantics).
6. Transvector and Fabric primitives for state merge/write/read rules.

## Event System (High-level)

Core event ideas already in project:
1. Event classes (`CRIT/HIGH/NORM/LOW`) with controlled drain budget.
2. Per-task subscriptions and fan-out routing.
3. Mailbox counters + overflow guard.
4. Source and payload tagging for delivered events.
5. Latency and deadline accounting with throttled violation logs.

This gives deterministic behavior under load instead of uncontrolled log/event flood.

## Repository Layout

Main directories:

```text
.
├─ src/                     # UEFI bootloader
├─ kernel/                  # no_std kernel
│  ├─ src/
│  │  ├─ main.rs
│  │  ├─ runtime.rs
│  │  ├─ kernel_state.rs
│  │  ├─ transvector.rs
│  │  ├─ transjector.rs
│  │  ├─ framebuffer.rs
│  │  ├─ serial.rs
│  │  ├─ log.rs
│  │  └─ demo.rs
├─ run.ps1                  # build + run orchestration (QEMU/OVMF/ESP)
└─ dist/                    # generated runtime artifacts/logs
```

## Build

Bootloader:

```powershell
cargo build --release
```

Kernel:

```powershell
cd kernel
cargo build --release
```

## Run (Recommended)

Use project runner script:

```powershell
.\run.ps1
```

It performs:
1. Release build of bootloader and kernel.
2. ESP layout preparation:
   `\EFI\BOOT\BOOTX64.EFI`
   `\EFI\BOOT\KERNEL.ELF`
3. QEMU launch with OVMF firmware.
4. Log capture into `dist\logs\`.

## Status

Current state is a working foundation, not a general-purpose OS yet:
1. Boot path is operational in QEMU + OVMF.
2. Kernel handoff and runtime are alive and instrumented.
3. Event-driven architecture is in active incremental milestones.
4. Portability plan is oriented around HAL + Transjector normalization, keeping runtime semantics stable across platforms.

## License

This project is proprietary and distributed under the terms of `LICENSE`.
Unauthorized use, copying, modification, and redistribution are prohibited.
