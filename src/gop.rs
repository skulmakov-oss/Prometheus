use crate::handoff::Framebuffer;
use uefi::proto::console::gop::{GraphicsOutput, PixelFormat};
use uefi::table::{Boot, SystemTable};
use uefi::Status;

fn map_format(fmt: PixelFormat) -> u32 {
    match fmt {
        PixelFormat::Rgb => 0,
        PixelFormat::Bgr => 1,
        PixelFormat::Bitmask => 2,
        PixelFormat::BltOnly => 3,
    }
}

pub fn init(st: &mut SystemTable<Boot>) -> Result<Framebuffer, Status> {
    let bs = st.boot_services();
    let gop_handle = bs
        .get_handle_for_protocol::<GraphicsOutput>()
        .map_err(|err| err.status())?;
    let mut gop = bs
        .open_protocol_exclusive::<GraphicsOutput>(gop_handle)
        .map_err(|err| err.status())?;

    let info = gop.current_mode_info();
    let (width, height) = info.resolution();
    let mut fb = gop.frame_buffer();

    Ok(Framebuffer {
        base: fb.as_mut_ptr() as u64,
        size: fb.size() as u64,
        width: width as u32,
        height: height as u32,
        stride: info.stride() as u32,
        format: map_format(info.pixel_format()),
    })
}
