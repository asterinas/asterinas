// SPDX-License-Identifier: MPL-2.0

/// Individual pixel data containing raw channel values.
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct Pixel {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
}

/// Pixel format that defines the memory layout of each pixel in the framebuffer.
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum PixelFormat {
    /// Each pixel uses 8 bits to represent its grayscale intensity, ranging from 0 (black) to 255 (white).
    Grayscale8,
    /// Each pixel uses 16 bits, with 5 bits for Red, 6 bits for Green, and 5 bits for Blue.
    Rgb565,
    /// Each pixel uses 24 bits, with 8 bits for Red, 8 bits for Green, and 8 bits for Blue.
    Rgb888,
    /// Each pixel uses 32 bits, with 8 bits for Blue, 8 bits for Green, 8 bits for Red, and 8 bits reserved.
    BgrReserved,
}

/// A rendered pixel in a specific format.
#[derive(Debug, Copy, Clone)]
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
                buf[0..2].copy_from_slice(&rgb565.to_be_bytes());
                RenderedPixel { buf, len: 2 }
            }
            PixelFormat::Rgb888 => {
                buf[0] = self.red;
                buf[1] = self.green;
                buf[2] = self.blue;
                RenderedPixel { buf, len: 3 }
            }
            PixelFormat::BgrReserved => {
                buf[0] = self.blue;
                buf[1] = self.green;
                buf[2] = self.red;
                RenderedPixel { buf, len: 4 }
            }
        }
    }
}

impl PixelFormat {
    /// Returns the number of bytes per pixel (color depth).
    pub fn nbytes(&self) -> usize {
        match self {
            PixelFormat::Grayscale8 => 1,
            PixelFormat::Rgb565 => 2,
            PixelFormat::Rgb888 => 3,
            PixelFormat::BgrReserved => 4,
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
