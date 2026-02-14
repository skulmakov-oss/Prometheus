use crate::font;

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

pub struct FramebufferWriter {
    fb: Framebuffer,
    cursor_x: usize,
    cursor_y: usize,
    fg: u32,
    bg: u32,
}

impl FramebufferWriter {
    pub const fn new(fb: Framebuffer) -> Self {
        Self {
            fb,
            cursor_x: 8,
            cursor_y: 8,
            fg: 0x00FFFFFF,
            bg: 0x00000000,
        }
    }

    pub fn clear(&mut self) {
        let height = self.fb.height as usize;
        let width = self.fb.width as usize;
        let stride = self.fb.stride as usize;

        for y in 0..height {
            for x in 0..width {
                self.put_pixel(x, y, self.bg);
            }
            for x in width..stride {
                self.put_pixel(x, y, self.bg);
            }
        }
    }

    pub fn write_line(&mut self, s: &str) {
        for b in s.bytes() {
            self.write_byte(b);
        }
        self.new_line();
    }

    fn write_byte(&mut self, b: u8) {
        const GLYPH_W: usize = 8;
        const GLYPH_H: usize = 16;

        if self.cursor_x + GLYPH_W >= self.fb.width as usize {
            self.new_line();
        }

        if self.cursor_y + GLYPH_H >= self.fb.height as usize {
            return;
        }

        let glyph = font::glyph(b);

        for (row, bits) in glyph.iter().enumerate() {
            for col in 0..8 {
                let mask = 1 << (7 - col);
                let color = if (bits & mask) != 0 { self.fg } else { self.bg };
                self.put_pixel(self.cursor_x + col, self.cursor_y + row, color);
            }
        }

        self.cursor_x += GLYPH_W;
    }

    fn new_line(&mut self) {
        self.cursor_x = 8;
        self.cursor_y += 18;
    }

    fn put_pixel(&mut self, x: usize, y: usize, rgb: u32) {
        let stride = self.fb.stride as usize;
        let pixel_index = y.saturating_mul(stride).saturating_add(x);
        let byte_offset = pixel_index.saturating_mul(4);

        if byte_offset + 3 >= self.fb.size as usize {
            return;
        }

        let (r, g, b) = (((rgb >> 16) & 0xFF) as u8, ((rgb >> 8) & 0xFF) as u8, (rgb & 0xFF) as u8);

        unsafe {
            let ptr = (self.fb.base as *mut u8).add(byte_offset);
            match self.fb.format {
                1 => {
                    ptr.write_volatile(b);
                    ptr.add(1).write_volatile(g);
                    ptr.add(2).write_volatile(r);
                    ptr.add(3).write_volatile(0);
                }
                _ => {
                    ptr.write_volatile(r);
                    ptr.add(1).write_volatile(g);
                    ptr.add(2).write_volatile(b);
                    ptr.add(3).write_volatile(0);
                }
            }
        }
    }
}
