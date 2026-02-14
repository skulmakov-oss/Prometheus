use core::mem::MaybeUninit;
use core::ptr::{addr_of, addr_of_mut};

use crate::handoff::BootInfo;

static mut BOOT_INFO_SLOT: MaybeUninit<BootInfo> = MaybeUninit::uninit();

pub fn store_boot_info(info: BootInfo) -> *const BootInfo {
    unsafe {
        addr_of_mut!(BOOT_INFO_SLOT).write(MaybeUninit::new(info));
        addr_of!(BOOT_INFO_SLOT).cast::<BootInfo>()
    }
}
