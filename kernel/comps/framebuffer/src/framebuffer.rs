// SPDX-License-Identifier: MPL-2.0

use alloc::{sync::Arc, vec::Vec};

use ostd::{
    boot::boot_info,
    io::IoMem,
    mm::{HasSize, VmIo},
    sync::Mutex,
    Error, Result,
};
use spin::Once;

use crate::{Pixel, PixelFormat, RenderedPixel};

/// Maximum number of colormap entries (standard 8-bit palette)
pub const MAX_CMAP_SIZE: usize = 256;

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
    width: usize,
    height: usize,
    line_size: usize,
    pixel_format: PixelFormat,
    cmap: Mutex<FbCmap>,
}

/// A single entry in the color map with 16-bit color values.
///
/// Linux framebuffer colormap uses 16-bit values (0-65535) for each color channel
/// to support high precision color mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColorMapEntry {
    /// Red color value (16-bit)
    pub red: u16,
    /// Green color value (16-bit)
    pub green: u16,
    /// Blue color value (16-bit)
    pub blue: u16,
    /// Transparency value (16-bit)
    pub transp: u16,
}

/// Internal framebuffer colormap structure.
#[derive(Debug, Clone)]
struct FbCmap {
    /// Color map entries
    entries: Vec<ColorMapEntry>,
}

pub static FRAMEBUFFER: Once<Arc<FrameBuffer>> = Once::new();

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

        let default_cmap = FbCmap {
            entries: Vec::new(),
        };

        FrameBuffer {
            io_mem,
            width: framebuffer_arg.width,
            height: framebuffer_arg.height,
            line_size,
            pixel_format,
            cmap: Mutex::new(default_cmap),
        }
    };

    framebuffer.clear();
    FRAMEBUFFER.call_once(|| Arc::new(framebuffer));
}

impl FrameBuffer {
    /// Returns the width of the framebuffer in pixels.
    pub fn width(&self) -> usize {
        self.width
    }

    /// Returns the height of the framebuffer in pixels.
    pub fn height(&self) -> usize {
        self.height
    }

    /// Returns the line size of the framebuffer in bytes.
    pub fn line_size(&self) -> usize {
        self.line_size
    }

    /// Returns a reference to the `IoMem` instance of the framebuffer.
    pub fn io_mem(&self) -> &IoMem {
        &self.io_mem
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
        let frame = alloc::vec![0u8; self.io_mem().size()];
        self.write_bytes_at(0, &frame).unwrap();
    }

    /// Sets color map entries starting from the given index.
    ///
    /// For efifb devices, hardware color map is not supported, so we maintain
    /// an in-memory map for software emulation.
    pub fn set_color_map(&self, start: usize, entries: &[ColorMapEntry]) -> Result<()> {
        if start > MAX_CMAP_SIZE || entries.len() > MAX_CMAP_SIZE - start {
            return Err(Error::InvalidArgs);
        }

        let mut cmap = self.cmap.lock();
        let required_len = start + entries.len();

        // Ensure the colormap has enough space
        if cmap.entries.len() < required_len {
            cmap.entries.resize(
                required_len,
                ColorMapEntry {
                    red: 0,
                    green: 0,
                    blue: 0,
                    transp: 0,
                },
            );
        }

        // Copy the entries
        cmap.entries[start..start + entries.len()].copy_from_slice(entries);

        Ok(())
    }

    /// Gets color map entries from the given range.
    pub fn get_color_map(&self, start: usize, len: usize) -> Option<Vec<ColorMapEntry>> {
        let cmap = self.cmap.lock();

        if start >= cmap.entries.len() || len > cmap.entries.len() - start {
            return None;
        }

        Some(cmap.entries[start..start + len].to_vec())
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
