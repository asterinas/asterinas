// SPDX-License-Identifier: MPL-2.0

//! The framebuffer of Asterinas.

use alloc::{vec, vec::Vec};
use core::{
    fmt::{Arguments, Result, Write},
    ops::IndexMut,
};

use font8x8::UnicodeFonts;
use spin::Once;

use crate::{
    boot,
    io_mem::IoMem,
    mm::{CachePolicy, PageFlags, VmIo},
    sync::SpinLock,
};

pub(crate) static WRITER: Once<SpinLock<Writer>> = Once::new();

pub(crate) fn init() {
    let mut writer = {
        let framebuffer = boot::framebuffer_arg();
        if framebuffer.address == 0 {
            log::debug!("Framebuffer base address is zero");
            return;
        }

        log::debug!("Found framebuffer:{:?}", framebuffer);
        let fb_base = framebuffer.address;
        let fb_len = (framebuffer.width * framebuffer.height * framebuffer.bpp).div_ceil(8);
        let io_mem = unsafe {
            IoMem::new(
                fb_base..fb_base + fb_len,
                PageFlags::RW,
                CachePolicy::Writeback,
            )
        };
        let buffer: Vec<u8> = vec![0; fb_len];

        Writer {
            io_mem,
            x_pos: 0,
            y_pos: 0,
            bytes_per_pixel: (framebuffer.bpp / 8) as usize,
            width: framebuffer.width as usize,
            height: framebuffer.height as usize,
            buffer: buffer.leak(),
        }
    };
    writer.clear();

    WRITER.call_once(|| SpinLock::new(writer));
}

pub(crate) struct Writer {
    io_mem: IoMem,
    /// FIXME: remove buffer. The meaning of buffer is to facilitate the various operations of framebuffer
    buffer: &'static mut [u8],

    bytes_per_pixel: usize,
    width: usize,
    height: usize,

    x_pos: usize,
    y_pos: usize,
}

impl Writer {
    fn newline(&mut self) {
        // Reserve the last blank line.
        if self.y_pos >= self.height() - 16 {
            self.shift_lines_up();
        }
        self.y_pos += 8;
        self.carriage_return();
    }

    fn carriage_return(&mut self) {
        self.x_pos = 0;
    }

    /// Erases all text on the screen.
    pub fn clear(&mut self) {
        self.x_pos = 0;
        self.y_pos = 0;
        self.buffer.fill(0);
        self.io_mem.write_bytes(0, self.buffer).unwrap();
    }

    /// Everything moves up one letter in size.
    fn shift_lines_up(&mut self) {
        let offset = self.width() * self.bytes_per_pixel * 8;
        self.buffer.copy_within(offset.., 0);
        self.io_mem.write_bytes(0, self.buffer).unwrap();
        self.y_pos -= 8;
    }

    fn width(&self) -> usize {
        self.width
    }

    fn height(&self) -> usize {
        self.height
    }

    fn write_char(&mut self, c: char) {
        match c {
            '\n' => self.newline(),
            '\r' => self.carriage_return(),
            c => {
                if self.x_pos >= self.width {
                    self.newline();
                }
                let rendered = font8x8::BASIC_FONTS
                    .get(c)
                    .expect("character not found in basic font");
                self.write_rendered_char(rendered);
            }
        }
    }

    fn write_rendered_char(&mut self, rendered_char: [u8; 8]) {
        for (y, byte) in rendered_char.iter().enumerate() {
            for (x, bit) in (0..8).enumerate() {
                let on = *byte & (1 << bit) != 0;
                self.write_pixel(self.x_pos + x, self.y_pos + y, on);
            }
        }
        self.x_pos += 8;
    }

    fn write_pixel(&mut self, x: usize, y: usize, on: bool) {
        let pixel_offset = y * self.width + x;
        let color = if on {
            [0x33, 0xff, 0x66, 0]
        } else {
            [0, 0, 0, 0]
        };
        let bytes_per_pixel = self.bytes_per_pixel;
        let byte_offset = pixel_offset * bytes_per_pixel;
        self.buffer
            .index_mut(byte_offset..(byte_offset + bytes_per_pixel))
            .copy_from_slice(&color[..bytes_per_pixel]);
        self.io_mem
            .write_bytes(byte_offset, &color[..bytes_per_pixel])
            .unwrap();
    }

    /// Writes the given ASCII string to the buffer.
    ///
    /// Wraps lines at `BUFFER_WIDTH`. Supports the `\n` newline character. Does **not**
    /// support strings with non-ASCII characters, since they can't be printed in the VGA text
    /// mode.
    fn write_string(&mut self, s: &str) {
        for char in s.chars() {
            self.write_char(char);
        }
    }
}

impl Write for Writer {
    fn write_str(&mut self, s: &str) -> Result {
        self.write_string(s);
        Ok(())
    }
}

/// Prints the given formatted string to the framebuffer.
pub fn print(args: Arguments) {
    let Some(writer) = WRITER.get() else {
        return;
    };

    writer.disable_irq().lock().write_fmt(args).unwrap();
}
