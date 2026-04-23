use uefi::table::boot::{AllocateType, BootServices, MemoryType};

const PT_LOAD: u32 = 1;
const ELF_MAGIC: [u8; 4] = [0x7F, b'E', b'L', b'F'];

#[derive(Clone, Copy)]
pub enum ElfLoadError {
    BadHeader,
    BadProgramHeader,
    SegmentLayout,
    SegmentOverflow,
    SegmentAlloc,
    SegmentSourceBounds,
    NoLoadSegments,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Elf64Header {
    e_ident: [u8; 16],
    e_type: u16,
    e_machine: u16,
    e_version: u32,
    e_entry: u64,
    e_phoff: u64,
    e_shoff: u64,
    e_flags: u32,
    e_ehsize: u16,
    e_phentsize: u16,
    e_phnum: u16,
    e_shentsize: u16,
    e_shnum: u16,
    e_shstrndx: u16,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Elf64ProgramHeader {
    p_type: u32,
    p_flags: u32,
    p_offset: u64,
    p_vaddr: u64,
    p_paddr: u64,
    p_filesz: u64,
    p_memsz: u64,
    p_align: u64,
}

#[derive(Clone, Copy)]
pub struct ElfLoadInfo {
    pub entry: u64,
    pub load_base: u64,
}

pub fn load_elf64(image: &[u8], bs: &BootServices) -> Result<u64, ElfLoadError> {
    Ok(load_elf64_with_base(image, bs)?.entry)
}

pub fn load_elf64_with_base(image: &[u8], bs: &BootServices) -> Result<ElfLoadInfo, ElfLoadError> {
    let ehdr = parse_header(image)?;
    validate_header(&ehdr)?;

    let mut min_page = u64::MAX;
    let mut max_page = 0u64;
    let mut saw_load = false;

    for idx in 0..ehdr.e_phnum as usize {
        let ph = parse_program_header(image, &ehdr, idx)?;
        if ph.p_type != PT_LOAD {
            continue;
        }
        saw_load = true;
        if ph.p_memsz < ph.p_filesz {
            return Err(ElfLoadError::SegmentLayout);
        }
        let seg_start = if ph.p_paddr != 0 { ph.p_paddr } else { ph.p_vaddr };
        let seg_end = seg_start
            .checked_add(ph.p_memsz)
            .ok_or(ElfLoadError::SegmentOverflow)?;
        let page_start = align_down(seg_start, 4096);
        let page_end = align_up(seg_end, 4096);
        if page_start < min_page {
            min_page = page_start;
        }
        if page_end > max_page {
            max_page = page_end;
        }
    }

    if !saw_load {
        return Err(ElfLoadError::NoLoadSegments);
    }

    if max_page <= min_page {
        return Err(ElfLoadError::SegmentLayout);
    }
    let pages = ((max_page - min_page) / 4096) as usize;
    bs.allocate_pages(AllocateType::Address(min_page), MemoryType::LOADER_DATA, pages)
        .map_err(|_| ElfLoadError::SegmentAlloc)?;

    for idx in 0..ehdr.e_phnum as usize {
        let ph = parse_program_header(image, &ehdr, idx)?;
        if ph.p_type != PT_LOAD {
            continue;
        }
        load_segment(image, &ph)?;
    }

    Ok(ElfLoadInfo {
        entry: ehdr.e_entry,
        load_base: min_page,
    })
}

fn parse_header(image: &[u8]) -> Result<Elf64Header, ElfLoadError> {
    if image.len() < core::mem::size_of::<Elf64Header>() {
        return Err(ElfLoadError::BadHeader);
    }

    let ptr = image.as_ptr() as *const Elf64Header;
    let hdr = unsafe { *ptr };
    Ok(hdr)
}

fn validate_header(hdr: &Elf64Header) -> Result<(), ElfLoadError> {
    if hdr.e_ident[0..4] != ELF_MAGIC {
        return Err(ElfLoadError::BadHeader);
    }

    if hdr.e_ident[4] != 2 || hdr.e_ident[5] != 1 {
        return Err(ElfLoadError::BadHeader);
    }

    if hdr.e_machine != 0x3E {
        return Err(ElfLoadError::BadHeader);
    }

    Ok(())
}

fn parse_program_header(image: &[u8], hdr: &Elf64Header, idx: usize) -> Result<Elf64ProgramHeader, ElfLoadError> {
    let off = hdr.e_phoff as usize + idx * hdr.e_phentsize as usize;
    let end = off + core::mem::size_of::<Elf64ProgramHeader>();
    if end > image.len() {
        return Err(ElfLoadError::BadProgramHeader);
    }

    let ptr = unsafe { image.as_ptr().add(off) as *const Elf64ProgramHeader };
    Ok(unsafe { *ptr })
}

fn load_segment(image: &[u8], ph: &Elf64ProgramHeader) -> Result<(), ElfLoadError> {
    if ph.p_memsz < ph.p_filesz {
        return Err(ElfLoadError::SegmentLayout);
    }

    let seg_start = if ph.p_paddr != 0 { ph.p_paddr } else { ph.p_vaddr };
    let _seg_end = seg_start
        .checked_add(ph.p_memsz)
        .ok_or(ElfLoadError::SegmentOverflow)?;

    let src_off = ph.p_offset as usize;
    let src_end = src_off + ph.p_filesz as usize;
    if src_end > image.len() {
        return Err(ElfLoadError::SegmentSourceBounds);
    }

    unsafe {
        let dst = seg_start as *mut u8;
        let src = image.as_ptr().add(src_off);
        core::ptr::copy_nonoverlapping(src, dst, ph.p_filesz as usize);

        let bss = dst.add(ph.p_filesz as usize);
        core::ptr::write_bytes(bss, 0, (ph.p_memsz - ph.p_filesz) as usize);
    }

    Ok(())
}

const fn align_down(value: u64, align: u64) -> u64 {
    value & !(align - 1)
}

const fn align_up(value: u64, align: u64) -> u64 {
    (value + align - 1) & !(align - 1)
}


