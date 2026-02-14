#![no_std]
#![no_main]

extern crate alloc;

mod elf;
mod fs;
mod gop;
mod handoff;
mod memory;

use core::fmt::Write;
use handoff::{BootInfo, KernelEntry};
use uefi::prelude::*;
use uefi::table::boot::MemoryType;
use uefi::table::cfg::ACPI2_GUID;

fn log_line(st: &mut SystemTable<Boot>, msg: &str) {
    let _ = st.stdout().write_str(msg);
    let _ = st.stdout().write_str("\r\n");
}

fn log_status(st: &mut SystemTable<Boot>, label: &str, status: Status) {
    let _ = writeln!(st.stdout(), "{}: {:?}", label, status);
}

#[entry]
fn efi_main(image: Handle, mut st: SystemTable<Boot>) -> Status {
    #[allow(deprecated)]
    if uefi_services::init(&mut st).is_err() {
        return Status::ABORTED;
    }

    log_line(&mut st, "VectorOS Bootloader");

    let framebuffer = match gop::init(&mut st) {
        Ok(fb) => {
            log_line(&mut st, "GOP OK");
            fb
        }
        Err(status) => {
            log_status(&mut st, "GOP ERR", status);
            return status;
        }
    };

    let kernel = match fs::load_kernel_file(image, &mut st) {
        Ok(file) => {
            log_line(&mut st, "Disk OK");
            file
        }
        Err(status) => {
            log_status(&mut st, "Disk ERR", status);
            return status;
        }
    };

    let entry_addr = match elf::load_elf64(kernel, st.boot_services()) {
        Ok(entry) => {
            log_line(&mut st, "Kernel loaded");
            entry
        }
        Err(_) => {
            log_status(&mut st, "Kernel ERR", Status::LOAD_ERROR);
            return Status::LOAD_ERROR;
        }
    };

    let rsdp_addr = st
        .config_table()
        .iter()
        .find(|entry| entry.guid == ACPI2_GUID)
        .map(|entry| entry.address as u64)
        .unwrap_or(0);

    let desc_size = st.boot_services().memory_map_size().entry_size;
    log_line(&mut st, "ExitBootServices OK");
    log_line(&mut st, "Jump to kernel");

    let (_runtime_st, mmap) = st.exit_boot_services(MemoryType::LOADER_DATA);
    let mut mmap_entries = mmap.entries();
    let mmap_len = mmap_entries.len();
    let mmap_ptr = mmap_entries
        .next()
        .map(|d| d as *const _ as *const u8)
        .unwrap_or(core::ptr::null());

    let boot_info = BootInfo {
        magic: handoff::BOOT_MAGIC,
        framebuffer,
        memory_map: handoff::MemoryMapInfo {
            map: mmap_ptr,
            size: mmap_len * desc_size,
            desc_size,
        },
        rsdp_addr,
    };

    let boot_info_ptr = memory::store_boot_info(boot_info);

    let entry: KernelEntry = unsafe { core::mem::transmute(entry_addr as usize) };
    entry(boot_info_ptr)
}
