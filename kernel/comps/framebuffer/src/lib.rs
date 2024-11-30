// SPDX-License-Identifier: MPL-2.0

//! The framebuffer of Asterinas.
#![no_std]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::{vec, vec::Vec};
use core::{
    fmt,
    ops::{Index, IndexMut},
};

use component::{init_component, ComponentInitError};
use font8x8::UnicodeFonts;
use ostd::{
    boot::{self, memory_region::MemoryRegionType, memory_regions},
    io_mem::IoMem,
    mm::{VmIo, PAGE_SIZE},
    sync::SpinLock,
};
use spin::Once;

#[init_component]
fn framebuffer_init() -> Result<(), ComponentInitError> {
    init();
    Ok(())
}

pub(crate) static WRITER: Once<SpinLock<Writer>> = Once::new();

// ignore the warnings since we use the `todo!` macro.
#[allow(unused_variables)]
#[allow(unreachable_code)]
#[allow(clippy::diverging_sub_expression)]
pub(crate) fn init() {
    let mut writer = {
        let framebuffer = boot::framebuffer_arg();
        let mut size = 0;
        for region in memory_regions() {
            if region.typ() == MemoryRegionType::Framebuffer {
                size = region.len();
            }
        }

        let page_size = size / PAGE_SIZE;

        let start_paddr = framebuffer.address;
        let io_mem = todo!("IoMem is private for components now, should fix it.");

        let mut buffer: Vec<u8> = vec![0; size];
        log::debug!("Found framebuffer:{:?}", framebuffer);

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
        self.y_pos += 8;
        self.carriage_return();
    }

    fn carriage_return(&mut self) {
        self.x_pos = 0;
    }

    /// Erases all text on the screen
    pub fn clear(&mut self) {
        self.x_pos = 0;
        self.y_pos = 0;
        self.buffer.fill(0);
        self.io_mem.write_bytes(0, self.buffer).unwrap();
    }

    /// Everything moves up one letter in size
    fn shift_lines_up(&mut self) {
        let offset = self.bytes_per_pixel * 8;
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
                if self.x_pos >= self.width() {
                    self.newline();
                }
                while self.y_pos >= (self.height() - 8) {
                    self.shift_lines_up();
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
            .write_bytes(
                byte_offset,
                self.buffer
                    .index(byte_offset..(byte_offset + bytes_per_pixel)),
            )
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

impl fmt::Write for Writer {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.write_string(s);
        Ok(())
    }
}

/// Like the `print!` macro in the standard library, but prints to the VGA text buffer.
#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => ($crate::_print(format_args!($($arg)*)));
}

/// Like the `println!` macro in the standard library, but prints to the VGA text buffer.
#[macro_export]
macro_rules! println {
    () => ($crate::print!("\n"));
    ($($arg:tt)*) => ($crate::print!("{}\n", format_args!($($arg)*)));
}

/// Prints the given formatted string to the VGA text buffer
/// through the global `WRITER` instance.
#[doc(hidden)]
pub fn _print(args: fmt::Arguments) {
    use core::fmt::Write;

    WRITER
        .get()
        .unwrap()
        .disable_irq()
        .lock()
        .write_fmt(args)
        .unwrap();
}
