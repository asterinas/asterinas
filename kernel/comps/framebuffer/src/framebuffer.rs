// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;
use core::sync::atomic::{AtomicU32, Ordering};

use ostd::{
    boot::boot_info,
    io::IoMem,
    mm::{HasSize, VmIo},
    Pod, Result,
};
use spin::Once;

use crate::{Pixel, PixelFormat, RenderedPixel};

/// Maximum number of pseudo palette entries (for efifb compatibility)
const PSEUDO_PALETTE_SIZE: usize = 16;

/// Framebuffer colormap structure for kernel use.
#[derive(Debug, Clone)]
pub struct FbCmap {
    /// Starting offset in colormap
    pub start: u32,
    /// Number of colormap entries
    pub len: u32,
    /// Red color values (16-bit)
    pub red: alloc::vec::Vec<u16>,
    /// Green color values (16-bit)
    pub green: alloc::vec::Vec<u16>,
    /// Blue color values (16-bit)
    pub blue: alloc::vec::Vec<u16>,
    /// Transparency values (16-bit), optional
    pub transp: Option<alloc::vec::Vec<u16>>,
}

/// Bitfield structure describing color channel layout.
///
/// This structure is aligned with Linux's `fb_bitfield` for system call compatibility.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Default)]
pub struct FbBitfield {
    /// Bit offset of the field
    pub offset: u32,
    /// Length of the field in bits
    pub length: u32,
    /// Most significant bit position (0 = left, 1 = right)
    pub msb_right: u32,
}

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
    /// Pseudo palette for software colormap emulation (16 entries for efifb)
    pseudo_palette: [AtomicU32; PSEUDO_PALETTE_SIZE],
    /// Current colormap
    cmap: spin::Mutex<FbCmap>,
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
            start: 0,
            len: 0,
            red: alloc::vec::Vec::new(),
            green: alloc::vec::Vec::new(),
            blue: alloc::vec::Vec::new(),
            transp: None,
        };

        FrameBuffer {
            io_mem,
            width: framebuffer_arg.width,
            height: framebuffer_arg.height,
            line_size,
            pixel_format,
            pseudo_palette: [const { AtomicU32::new(0) }; PSEUDO_PALETTE_SIZE],
            cmap: spin::Mutex::new(default_cmap),
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

    /// Returns a reference to the `IoMem` instance of the framebuffer.
    pub fn io_mem(&self) -> &IoMem {
        &self.io_mem
    }

    /// Returns the pixel format of the framebuffer.
    pub fn pixel_format(&self) -> PixelFormat {
        self.pixel_format
    }

    /// Returns a reference to the pseudo palette.
    pub fn pseudo_palette(&self) -> &[AtomicU32; PSEUDO_PALETTE_SIZE] {
        &self.pseudo_palette
    }

    /// Returns a reference to the colormap.
    pub fn cmap(&self) -> &spin::Mutex<FbCmap> {
        &self.cmap
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

    /// Sets a single color register in the pseudo palette
    ///
    /// For efifb devices, hardware colormap is not supported, so we maintain a
    /// pseudo palette for software emulation. This function implements
    /// software colormap emulation similar to Linux's efifb_setcolreg.
    pub fn set_color_regs(
        &self,
        regno: u32,
        red: u16,
        green: u16,
        blue: u16,
        _transp: u16,
        cmap_len: u32,
    ) -> Result<i32> {
        if regno >= cmap_len {
            return Ok(1); // Return 1 for invalid regno, matching Linux behavior
        }

        let pixel_format = self.pixel_format();
        let (red_bf, green_bf, blue_bf, _transp_bf) = Self::pixel_format_to_bitfields(pixel_format);

        // For pseudo palette (first 16 entries), store the computed pixel value
        if regno < PSEUDO_PALETTE_SIZE as u32 {
            // Shift color values down to hardware capabilities
            let red_val = (red as u32) >> (16 - red_bf.length);
            let green_val = (green as u32) >> (16 - green_bf.length);
            let blue_val = (blue as u32) >> (16 - blue_bf.length);

            // Combine into final pixel value based on bitfield offsets
            let pixel_value = (red_val << red_bf.offset)
                | (green_val << green_bf.offset)
                | (blue_val << blue_bf.offset);

            // Store in pseudo palette
            self.pseudo_palette[regno as usize].store(pixel_value, Ordering::Relaxed);
        }

        Ok(0)
    }

    /// Converts pixel format to framebuffer bitfields
    pub fn pixel_format_to_bitfields(
        pixel_format: PixelFormat,
    ) -> (FbBitfield, FbBitfield, FbBitfield, FbBitfield) {
        match pixel_format {
            PixelFormat::Grayscale8 => {
                let bitfield = FbBitfield {
                    offset: 0,
                    length: 8,
                    msb_right: 0,
                };
                (bitfield, bitfield, bitfield, FbBitfield::default())
            }
            PixelFormat::Rgb565 => (
                FbBitfield {
                    offset: 11,
                    length: 5,
                    msb_right: 0,
                },
                FbBitfield {
                    offset: 5,
                    length: 6,
                    msb_right: 0,
                },
                FbBitfield {
                    offset: 0,
                    length: 5,
                    msb_right: 0,
                },
                FbBitfield::default(),
            ),
            PixelFormat::Rgb888 => (
                FbBitfield {
                    offset: 16,
                    length: 8,
                    msb_right: 0,
                },
                FbBitfield {
                    offset: 8,
                    length: 8,
                    msb_right: 0,
                },
                FbBitfield {
                    offset: 0,
                    length: 8,
                    msb_right: 0,
                },
                FbBitfield::default(),
            ),
            PixelFormat::BgrReserved => (
                FbBitfield {
                    offset: 16,
                    length: 8,
                    msb_right: 0,
                },
                FbBitfield {
                    offset: 8,
                    length: 8,
                    msb_right: 0,
                },
                FbBitfield {
                    offset: 0,
                    length: 8,
                    msb_right: 0,
                },
                FbBitfield {
                    offset: 24,
                    length: 8,
                    msb_right: 0,
                },
            ),
        }
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
