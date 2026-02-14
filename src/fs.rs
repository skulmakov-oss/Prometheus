use alloc::boxed::Box;
use uefi::proto::loaded_image::LoadedImage;
use uefi::proto::media::file::{File, FileAttribute, FileInfo, FileMode, FileType, RegularFile};
use uefi::proto::media::fs::SimpleFileSystem;
use uefi::table::boot;
use uefi::table::{Boot, SystemTable};
use uefi::{cstr16, Handle, Status};

pub fn load_kernel_file(image: Handle, st: &mut SystemTable<Boot>) -> Result<&'static [u8], Status> {
    let bs = st.boot_services();
    let loaded_image = bs
        .open_protocol_exclusive::<LoadedImage>(image)
        .map_err(|err| err.status())?;
    let mut fs = bs
        .open_protocol_exclusive::<SimpleFileSystem>(loaded_image.device().ok_or(Status::NOT_FOUND)?)
        .map_err(|err| err.status())?;

    let mut root = fs.open_volume().map_err(|err| err.status())?;
    let file = root
        .open(
            cstr16!("\\EFI\\BOOT\\KERNEL.ELF"),
            FileMode::Read,
            FileAttribute::empty(),
        )
        .map_err(|err| err.status())?;

    let mut kernel = match file.into_type().map_err(|err| err.status())? {
        FileType::Regular(file) => file,
        _ => return Err(Status::LOAD_ERROR),
    };

    let info = read_file_info(&mut kernel)?;
    let file_size = info.file_size() as usize;

    let ptr = bs
        .allocate_pool(boot::MemoryType::LOADER_DATA, file_size)
        .map_err(|err| err.status())?;

    let buffer = unsafe { core::slice::from_raw_parts_mut(ptr, file_size) };
    let read = kernel.read(buffer).map_err(|err| err.status())?;
    if read != file_size {
        return Err(Status::LOAD_ERROR);
    }

    Ok(buffer)
}

fn read_file_info(file: &mut RegularFile) -> Result<Box<FileInfo>, Status> {
    file.get_boxed_info::<FileInfo>().map_err(|err| err.status())
}
