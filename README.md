# VectorOS Bootloader v0

Minimal UEFI x86_64 bootloader in Rust (`no_std`) that loads ELF64 kernel and jumps to it with `BootInfo`.

## Build

```powershell
cargo build --release
```

Output EFI image:

```text
target\x86_64-unknown-uefi\release\vectoros-bootloader.efi
```

## FAT layout

Place files in a FAT image (or mounted ESP):

```text
\EFI\BOOT\BOOTX64.EFI   <- bootloader (rename vectoros-bootloader.efi)
\EFI\BOOT\KERNEL.ELF    <- kernel ELF64
```

## Run (QEMU + OVMF)

Example command (paths may differ):

```powershell
qemu-system-x86_64 `
  -machine q35 `
  -m 512M `
  -drive if=pflash,format=raw,readonly=on,file=OVMF_CODE.fd `
  -drive if=pflash,format=raw,file=OVMF_VARS.fd `
  -drive format=raw,file=fat:rw:esp
```

Expected bootloader log:

```text
VectorOS Bootloader
GOP OK
Disk OK
Kernel loaded
ExitBootServices OK
Jump to kernel
```
