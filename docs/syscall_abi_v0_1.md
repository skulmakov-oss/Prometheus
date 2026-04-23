# Syscall ABI v0.2

Status: minimal userland IPC layer on top of hardened v0.1 calls.

## Entry mechanism

- Architecture: x86_64
- Gate: `int 0x80` (IDT vector `0x80`)

## Register contract

- `rax`: syscall number (in), return value (out)
- `rdi`, `rsi`, `rdx`, `r10`, `r8`, `r9`: arguments

Return convention:

- `rax >= 0`: success (return value)
- `rax < 0`: error (`-errno` style)

## Syscall numbers

- `1`: `sys_log(ptr, len)`
- `2`: `sys_tick()`
- `3`: `sys_yield()`
- `4`: `sys_ipc_send(kind, arg0, arg1)`
- `5`: `sys_ipc_recv(ptr, len)`

## Error codes (fixed)

- `-22` `EINVAL`: invalid syscall number or bad generic argument
- `-14` `EFAULT`: invalid pointer/address
- `-34` `ERANGE`: invalid length for bounded copy/IPC recv
- `-16` `EBUSY`: per-tick budget exceeded or IPC mailbox full

## Limits and budgets

- `SYS_LOG_MAX = 256` bytes
- `SYS_LOG_BUDGET_BYTES_PER_TICK = 64` bytes
- `SYS_U_BUDGET_SYSCALLS_PER_TICK = 8` syscalls
- `IPC_RING_CAP = 8` messages per direction
- `IPC_DRAIN_BUDGET = 2` messages per tick

Budget scope:

- Log budget applies only to userland `sys_log`
- Syscall budget applies to all userland syscalls
- IPC drain budget applies to user->kernel mailbox processing

## IPC message format

```rust
#[repr(C)]
struct IpcMsg {
    kind: u32,
    arg0: u64,
    arg1: u64,
}
```

- Fixed size: `24` bytes on x86_64
- `kind=1`: `PING`
- `kind=2`: `PONG`

## Syscall semantics

### `sys_log(ptr, len)`

- `len == 0` -> `0`
- invalid pointer / non-canonical / `ptr + len` overflow -> `-EFAULT`
- `len > SYS_LOG_MAX` -> `-ERANGE`
- per-tick log budget exceeded -> `-EBUSY`
- otherwise bounded copy to stack buffer and serial write with `UL: ` prefix

### `sys_tick()`

- returns monotonic kernel tick (`u64`, same source as runtime `R10`)

### `sys_yield()`

- v0.2 behavior: noop, returns `0`
- emits one-time diagnostic line `UY0 yield noop` (first call only)

### `sys_ipc_send(kind, arg0, arg1)`

- `kind == 0` or `kind > u32::MAX` -> `-EINVAL`
- push to user->kernel bounded mailbox
- mailbox full -> `-EBUSY`
- success -> `0`

### `sys_ipc_recv(ptr, len)`

- `len != size_of(IpcMsg)` -> `-ERANGE`
- invalid pointer / non-canonical / `ptr + len` overflow -> `-EFAULT`
- non-blocking:
  - no message -> `0`
  - one message copied -> `24`

## Compatibility notes

- No EXOcode contract yet
- No ring3/MMU isolation yet
- ABI values above are stable for v0.2 and should not change without ABI version bump
