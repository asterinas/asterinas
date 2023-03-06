use alloc::slice;
use core::fmt;
use font8x8::UnicodeFonts;
use limine::{LimineFramebufferRequest, LimineMemoryMapEntryType};
use spin::Mutex;
use volatile::Volatile;

use crate::mm;

pub(crate) static WRITER: Mutex<Option<Writer>> = Mutex::new(None);
static FRAMEBUFFER_REUEST: LimineFramebufferRequest = LimineFramebufferRequest::new(0);

pub(crate) fn init() {
    let mut writer = {
        let response = FRAMEBUFFER_REUEST
            .get_response()
            .get()
            .expect("Not found framebuffer");
        assert_eq!(response.framebuffer_count, 1);
        let mut writer = None;
        let mut size = 0;
        for i in mm::MEMORY_REGIONS.get().unwrap().iter() {
            if i.typ == LimineMemoryMapEntryType::Framebuffer {
                size = i.len as usize;
            }
        }
        for i in response.framebuffers() {
            let buffer_mut = unsafe {
                let start = i.address.as_ptr().unwrap().addr();
                slice::from_raw_parts_mut(start as *mut u8, size)
            };

            writer = Some(Writer {
                buffer: Volatile::new(buffer_mut),
                x_pos: 0,
                y_pos: 0,
                bytes_per_pixel: i.bpp as usize,
                width: i.width as usize,
                height: i.height as usize,
            })
        }
        writer.unwrap()
    };
    writer.clear();

    // global writer should not be locked here
    let mut global_writer = WRITER.try_lock().unwrap();
    assert!(global_writer.is_none(), "Global writer already initialized");
    *global_writer = Some(writer);
}

pub(crate) struct Writer {
    buffer: Volatile<&'static mut [u8]>,

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
    }

    fn shift_lines_up(&mut self) {
        let offset = self.bytes_per_pixel * 8;
        self.buffer.copy_within(offset.., 0);
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
        let pixel_offset = y + x;
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
macro_rules! screen_print {
    ($($arg:tt)*) => ($crate::device::framebuffer::_print(format_args!($($arg)*)));
}

/// Like the `println!` macro in the standard library, but prints to the VGA text buffer.
#[macro_export]
macro_rules! screen_println {
    () => ($crate::screen_print!("\n"));
    ($($arg:tt)*) => ($crate::screen_print!("{}\n", format_args!($($arg)*)));
}

/// Prints the given formatted string to the VGA text buffer
/// through the global `WRITER` instance.
#[doc(hidden)]
pub fn _print(args: fmt::Arguments) {
    use core::fmt::Write;
    use x86_64::instructions::interrupts;

    interrupts::without_interrupts(|| {
        WRITER.lock().as_mut().unwrap().write_fmt(args).unwrap();
    });
}
