// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;

use ostd::{boot::boot_info, io::IoMem, mm::VmIo, Result};
use spin::Once;

use crate::{Pixel, PixelFormat, RenderedPixel};

/// The framebuffer used for text or graphical output.
///
/// # Notes
///
/// It is highly recommended to use a synchronization primitive, such as a `SpinLock`, to
/// lock the framebuffer before performing any operation on it.
/// Failing to properly synchronize access can result in corrupted framebuffer content
/// or unspecified behavior during rendering.
#[derive(Debug)]
pub struct FrameBuffer {
    io_mem: IoMem,
    base: usize,
    width: usize,
    height: usize,
    line_size: usize,
    pixel_format: PixelFormat,
}

pub static FRAMEBUFFER: Once<Arc<FrameBuffer>> = Once::new();

pub fn get_framebuffer_info() -> Option<Arc<FrameBuffer>> {
    FRAMEBUFFER.get().cloned()
}

pub(crate) fn init() {
    let Some(framebuffer_arg) = boot_info().framebuffer_arg else {
        log::warn!("Framebuffer not found");
        return;
    };

    if framebuffer_arg.address == 0 {
        log::error!("Framebuffer address is zero");
        return;
    }

    // FIXME: There are several pixel formats that have the same BPP. We lost the information
    // during the boot phase, so here we guess the pixel format on a best effort basis.
    let pixel_format = match framebuffer_arg.bpp {
        8 => PixelFormat::Grayscale8,
        16 => PixelFormat::Rgb565,
        24 => PixelFormat::Rgb888,
        32 => PixelFormat::BgrReserved,
        _ => {
            log::error!(
                "Unsupported framebuffer pixel format: {} bpp",
                framebuffer_arg.bpp
            );
            return;
        }
    };

    let framebuffer = {
        // FIXME: There can be more than `width` pixels per framebuffer line due to alignment
        // purposes. We need to collect this information during the boot phase.
        let line_size = framebuffer_arg
            .width
            .checked_mul(pixel_format.nbytes())
            .unwrap();
        let fb_size = framebuffer_arg.height.checked_mul(line_size).unwrap();

        let fb_base = framebuffer_arg.address;
        let io_mem = IoMem::acquire(fb_base..fb_base.checked_add(fb_size).unwrap()).unwrap();

        FrameBuffer {
            io_mem,
            base: framebuffer_arg.address,
            width: framebuffer_arg.width,
            height: framebuffer_arg.height,
            line_size,
            pixel_format,
        }
    };

    framebuffer.clear();
    FRAMEBUFFER.call_once(|| Arc::new(framebuffer));
}

impl FrameBuffer {
    /// Returns the size of the framebuffer in bytes.
    pub fn size(&self) -> usize {
        self.io_mem.length()
    }

    /// Returns the width of the framebuffer in pixels.
    pub fn width(&self) -> usize {
        self.width
    }

    /// Returns the height of the framebuffer in pixels.
    pub fn height(&self) -> usize {
        self.height
    }

    /// Get the IO memory of the framebuffer.
    pub fn io_mem(&self) -> &IoMem {
        // FIXME: Check the correctness of ownership
        &self.io_mem
    }

    /// Returns the physical address of the framebuffer.
    pub fn io_mem_base(&self) -> usize {
        self.base
    }

    /// Returns the resolution in pixels.
    pub fn resolution(&self) -> (usize, usize) {
        (self.width, self.height)
    }

    /// Returns the number of bytes per pixel (color depth).
    pub fn bytes_per_pixel(&self) -> usize {
        // self.bytes_per_pixel
        0
    }

    /// Returns the pixel format of the framebuffer.
    pub fn pixel_format(&self) -> PixelFormat {
        self.pixel_format
    }

    /// Renders the pixel according to the pixel format of the framebuffer.
    pub fn render_pixel(&self, pixel: Pixel) -> RenderedPixel {
        pixel.render(self.pixel_format)
    }

    /// Calculates the offset of a pixel at the specified position.
    pub fn calc_offset(&self, x: usize, y: usize) -> PixelOffset {
        PixelOffset {
            fb: self,
            offset: (x * self.pixel_format.nbytes() + y * self.line_size) as isize,
        }
    }

    /// Writes a pixel at the specified position.
    pub fn write_pixel_at(&self, offset: PixelOffset, pixel: RenderedPixel) -> Result<()> {
        self.io_mem.write_bytes(offset.as_usize(), pixel.as_slice())
    }

    /// Writes raw bytes at the specified offset.
    pub fn write_bytes_at(&self, offset: usize, bytes: &[u8]) -> Result<()> {
        self.io_mem.write_bytes(offset, bytes)
    }

    /// Clears the framebuffer with default color (black).
    pub fn clear(&self) {
        let frame = alloc::vec![0u8; self.size()];
        self.write_bytes_at(0, &frame).unwrap();
    }
}

/// The offset of a pixel in the framebuffer.
#[derive(Debug, Clone, Copy)]
pub struct PixelOffset<'a> {
    fb: &'a FrameBuffer,
    offset: isize,
}

impl PixelOffset<'_> {
    /// Adds the specified delta to the x coordinate.
    pub fn x_add(&mut self, x_delta: isize) {
        let delta = x_delta * self.fb.pixel_format.nbytes() as isize;
        self.offset += delta;
    }

    /// Adds the specified delta to the y coordinate.
    pub fn y_add(&mut self, y_delta: isize) {
        let delta = y_delta * self.fb.line_size as isize;
        self.offset += delta;
    }

    /// Returns the offset value as a `usize`.
    pub fn as_usize(&self) -> usize {
        self.offset as usize
    }
}
