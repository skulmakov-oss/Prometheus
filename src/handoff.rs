#[repr(C)]
pub struct BootInfo {
    pub magic: u64,
    pub framebuffer: Framebuffer,
    pub memory_map: MemoryMapInfo,
    pub rsdp_addr: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Framebuffer {
    pub base: u64,
    pub size: u64,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub format: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct MemoryMapInfo {
    pub map: *const u8,
    pub size: usize,
    pub desc_size: usize,
}

pub const BOOT_MAGIC: u64 = 0x534F544345565F56;
pub type KernelEntry = extern "sysv64" fn(*const BootInfo) -> !;
