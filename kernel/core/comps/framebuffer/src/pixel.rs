// SPDX-License-Identifier: MPL-2.0

use ostd::boot::FramebufferRgbLayout;

/// Individual pixel data containing raw channel values.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Pixel {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
}

/// Pixel format that defines the memory layout of each pixel in the framebuffer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PixelFormat {
    /// Each pixel uses 8 bits to represent its grayscale intensity, ranging from 0 (black) to 255 (white).
    Grayscale8,
    /// Each pixel uses 16 bits, with 5 bits for Red, 6 bits for Green, and 5 bits for Blue.
    Rgb565,
    /// Each pixel uses 24 bits, with 8 bits for Red, 8 bits for Green, and 8 bits for Blue.
    Rgb888,
    /// Each pixel uses 32 bits, with 8 bits for Blue, 8 bits for Green, 8 bits for Red, and 8 bits reserved.
    BgrReserved,
    /// Red, green, blue, and reserved bytes in memory order.
    RgbReserved,
    /// Blue, green, and red bytes in memory order.
    Bgr888,
}

/// A rendered pixel in a specific format.
#[derive(Clone, Copy, Debug)]
pub struct RenderedPixel {
    buf: [u8; 4],
    len: u8,
}

impl Pixel {
    /// Renders the pixel into a specific format.
    pub fn render(&self, format: PixelFormat) -> RenderedPixel {
        let mut buf = [0; 4];
        match format {
            PixelFormat::Grayscale8 => {
                // Calculate the grayscale value
                let red_weight = 77 * self.red as u16; // Equivalent to 0.299 * 256
                let green_weight = 150 * self.green as u16; // Equivalent to 0.587 * 256
                let blue_weight = 29 * self.blue as u16; // Equivalent to 0.114 * 256
                let grayscale = (red_weight + green_weight + blue_weight) >> 8; // Normalize to 0-255
                buf[0] = grayscale as u8;
                RenderedPixel { buf, len: 1 }
            }
            PixelFormat::Rgb565 => {
                let r = (self.red >> 3) as u16; // Red (5 bits)
                let g = (self.green >> 2) as u16; // Green (6 bits)
                let b = (self.blue >> 3) as u16; // Blue (5 bits)
                let rgb565 = (r << 11) | (g << 5) | b; // Combine into RGB565 format
                buf[0..2].copy_from_slice(&rgb565.to_le_bytes());
                RenderedPixel { buf, len: 2 }
            }
            PixelFormat::Rgb888 | PixelFormat::RgbReserved => {
                buf[0] = self.red;
                buf[1] = self.green;
                buf[2] = self.blue;
                RenderedPixel {
                    buf,
                    len: format.nbytes() as u8,
                }
            }
            PixelFormat::BgrReserved | PixelFormat::Bgr888 => {
                buf[0] = self.blue;
                buf[1] = self.green;
                buf[2] = self.red;
                RenderedPixel {
                    buf,
                    len: format.nbytes() as u8,
                }
            }
        }
    }
}

impl PixelFormat {
    pub(super) fn from_boot_layout(
        bits_per_pixel: usize,
        layout: Option<FramebufferRgbLayout>,
    ) -> Option<Self> {
        let Some(layout) = layout else {
            // Preserve compatibility with bootloaders that omit channel data.
            return match bits_per_pixel {
                8 => Some(Self::Grayscale8),
                16 => Some(Self::Rgb565),
                24 => Some(Self::Rgb888),
                32 => Some(Self::BgrReserved),
                _ => None,
            };
        };
        match (bits_per_pixel, layout.red(), layout.green(), layout.blue()) {
            (16, (11, 5), (5, 6), (0, 5)) => Some(Self::Rgb565),
            (24, (0, 8), (8, 8), (16, 8)) => Some(Self::Rgb888),
            (24, (16, 8), (8, 8), (0, 8)) => Some(Self::Bgr888),
            (32, (0, 8), (8, 8), (16, 8)) => Some(Self::RgbReserved),
            (32, (16, 8), (8, 8), (0, 8)) => Some(Self::BgrReserved),
            _ => None,
        }
    }

    /// Returns the number of bytes per pixel (color depth).
    pub fn nbytes(&self) -> usize {
        match self {
            PixelFormat::Grayscale8 => 1,
            PixelFormat::Rgb565 => 2,
            PixelFormat::Rgb888 | PixelFormat::Bgr888 => 3,
            PixelFormat::BgrReserved | PixelFormat::RgbReserved => 4,
        }
    }
}

impl RenderedPixel {
    /// Returns the number of bytes in the rendered pixel.
    pub fn nbytes(&self) -> usize {
        self.len as usize
    }

    /// Returns a slice to the rendered pixel data.
    pub fn as_slice(&self) -> &[u8] {
        &self.buf[..self.nbytes()]
    }
}

impl Pixel {
    pub const WHITE: Pixel = Pixel {
        red: 0xFF,
        green: 0xFF,
        blue: 0xFF,
    };
    pub const BLACK: Pixel = Pixel {
        red: 0x00,
        green: 0x00,
        blue: 0x00,
    };
}

#[cfg(ktest)]
mod test {
    use ostd::prelude::ktest;

    use super::*;

    #[ktest]
    fn renders_reported_rgb_layout() {
        let pixel = Pixel {
            red: 0x12,
            green: 0x34,
            blue: 0x56,
        };
        for (depth, red, blue, bytes) in [
            (24, 0, 16, &[0x12, 0x34, 0x56][..]),
            (24, 16, 0, &[0x56, 0x34, 0x12][..]),
            (32, 0, 16, &[0x12, 0x34, 0x56, 0][..]),
            (32, 16, 0, &[0x56, 0x34, 0x12, 0][..]),
        ] {
            let layout = FramebufferRgbLayout::new((red, 8), (8, 8), (blue, 8));
            let format = PixelFormat::from_boot_layout(depth, Some(layout)).unwrap();
            assert_eq!(pixel.render(format).as_slice(), bytes);
        }
        let layout = FramebufferRgbLayout::new((11, 5), (5, 6), (0, 5));
        let format = PixelFormat::from_boot_layout(16, Some(layout)).unwrap();
        let red = Pixel {
            red: 255,
            green: 0,
            blue: 0,
        };
        assert_eq!(red.render(format).as_slice(), &[0, 0xf8]);
    }

    #[ktest]
    fn rejects_unsupported_explicit_layout() {
        let overlapping = FramebufferRgbLayout::new((0, 8), (0, 8), (16, 8));
        let narrow_green = FramebufferRgbLayout::new((0, 8), (8, 7), (16, 8));
        for layout in [overlapping, narrow_green] {
            assert_eq!(PixelFormat::from_boot_layout(32, Some(layout)), None);
        }
        assert_eq!(
            PixelFormat::from_boot_layout(32, None),
            Some(PixelFormat::BgrReserved)
        );
    }
}
