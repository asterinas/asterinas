// SPDX-License-Identifier: MPL-2.0

//! Framebuffer geometry and color layout supplied by bootloaders.
//!
//! [`BootloaderFramebufferArg`] validates the memory extent used by boot-time
//! reservation and the framebuffer driver.
//! [`FramebufferRgbLayout`] preserves the channel metadata
//! for the driver to select a supported pixel format.

use core::ops::Range;

use crate::mm;

/// The position and width of each RGB channel, in bits from the least significant bit.
#[derive(Clone, Copy, Debug)]
pub struct FramebufferRgbLayout {
    red: (u8, u8),
    green: (u8, u8),
    blue: (u8, u8),
}

impl FramebufferRgbLayout {
    /// Creates a layout from `(bit_position, bit_width)` pairs.
    ///
    /// A layout describes firmware data; it need not be supported by a renderer.
    pub const fn new(red: (u8, u8), green: (u8, u8), blue: (u8, u8)) -> Self {
        Self { red, green, blue }
    }

    /// Returns the red channel's bit position and width.
    pub const fn red(&self) -> (u8, u8) {
        self.red
    }

    /// Returns the green channel's bit position and width.
    pub const fn green(&self) -> (u8, u8) {
        self.green
    }

    /// Returns the blue channel's bit position and width.
    pub const fn blue(&self) -> (u8, u8) {
        self.blue
    }
}

/// Validated framebuffer arguments supplied by a bootloader.
#[derive(Clone, Copy, Debug)]
pub struct BootloaderFramebufferArg {
    address: usize,
    width: usize,
    height: usize,
    bits_per_pixel: usize,
    pitch_bytes: usize,
    rgb_layout: Option<FramebufferRgbLayout>,
}

impl BootloaderFramebufferArg {
    pub(crate) fn new(
        address: usize,
        width: usize,
        height: usize,
        bits_per_pixel: usize,
        pitch_bytes: usize,
        rgb_layout: Option<FramebufferRgbLayout>,
    ) -> Option<Self> {
        if address == 0 || width == 0 || height == 0 || bits_per_pixel == 0 {
            return None;
        }
        let row_bytes = width.checked_mul(bits_per_pixel.div_ceil(8))?;
        if pitch_bytes < row_bytes {
            return None;
        }
        let end = address.checked_add(pitch_bytes.checked_mul(height)?)?;
        // Memory reservation and MMIO mapping round the end up to a page boundary.
        end.checked_add(mm::PAGE_SIZE - 1)?;
        Some(Self {
            address,
            width,
            height,
            bits_per_pixel,
            pitch_bytes,
            rgb_layout,
        })
    }

    /// Returns the physical address range, including scanline padding.
    pub fn physical_range(&self) -> Range<usize> {
        // Construction checks both the multiplication and the end address.
        self.address..self.address + self.pitch_bytes * self.height
    }

    /// Returns the width in pixels.
    pub fn width(&self) -> usize {
        self.width
    }

    /// Returns the height in pixels.
    pub fn height(&self) -> usize {
        self.height
    }

    /// Returns the storage depth in bits per pixel.
    pub fn bits_per_pixel(&self) -> usize {
        self.bits_per_pixel
    }

    /// Returns the scanline length in bytes, including padding.
    pub fn pitch_bytes(&self) -> usize {
        self.pitch_bytes
    }

    /// Returns the RGB layout, if the bootloader describes the channels.
    pub fn rgb_layout(&self) -> Option<FramebufferRgbLayout> {
        self.rgb_layout
    }
}

#[cfg(ktest)]
mod test {
    use super::*;
    use crate::{boot::memory_region::MemoryRegion, prelude::ktest};

    #[ktest]
    fn reserves_scanline_padding() {
        let fb = BootloaderFramebufferArg::new(0x1000, 3, 2, 32, 16, None).unwrap();
        assert_eq!(fb.physical_range(), 0x1000..0x1020);
        assert_eq!(fb.pitch_bytes(), 16);
        let region = MemoryRegion::framebuffer(&fb);
        assert_eq!(region.base(), 0x1000);
        assert_eq!(region.len(), 32);
    }

    #[ktest]
    fn rejects_invalid_geometry() {
        assert!(BootloaderFramebufferArg::new(0x1000, 3, 2, 32, 11, None).is_none());
        assert!(BootloaderFramebufferArg::new(0x1000, 0, 2, 32, 16, None).is_none());
        assert!(BootloaderFramebufferArg::new(0x1000, 3, 0, 32, 16, None).is_none());
        assert!(BootloaderFramebufferArg::new(0x1000, 3, 2, 0, 16, None).is_none());
        assert!(BootloaderFramebufferArg::new(0, 3, 2, 32, 16, None).is_none());
        assert!(BootloaderFramebufferArg::new(0x1000, usize::MAX, 2, 32, 16, None).is_none());
        assert!(BootloaderFramebufferArg::new(0x1000, 3, usize::MAX, 32, 16, None).is_none());
        assert!(BootloaderFramebufferArg::new(usize::MAX - 16, 3, 2, 32, 16, None).is_none());
        assert!(BootloaderFramebufferArg::new(usize::MAX - 16, 1, 1, 32, 4, None).is_none());

        // The last representable page boundary is still a valid exclusive end.
        let end = usize::MAX - (mm::PAGE_SIZE - 1);
        let fb = BootloaderFramebufferArg::new(end - 4, 1, 1, 32, 4, None).unwrap();
        assert_eq!(fb.physical_range(), end - 4..end);
    }
}
