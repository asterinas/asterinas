// SPDX-License-Identifier: MPL-2.0

use alloc::{sync::Arc, vec::Vec};

use aster_console::{AnyConsoleDevice, ConsoleCallback};
use font8x8::UnicodeFonts;
use ostd::{
    sync::{LocalIrqDisabled, SpinLock},
    Error, Result,
};
use spin::Once;

use crate::{FrameBuffer, Pixel, FRAMEBUFFER};

/// The font width in pixels when using `font8x8`.
const FONT_WIDTH: usize = 8;

/// The font height in pixels when using `font8x8`.
const FONT_HEIGHT: usize = 8;

/// A text console rendered onto the framebuffer.
#[derive(Debug)]
pub struct FramebufferConsole {
    state: SpinLock<ConsoleState, LocalIrqDisabled>,
}

pub static CONSOLE_NAME: &str = "Framebuffer-Console";

pub static FRAMEBUFFER_CONSOLE: Once<Arc<FramebufferConsole>> = Once::new();

pub(crate) fn init() {
    let Some(fb) = FRAMEBUFFER.get() else {
        log::warn!("Framebuffer not initialized");
        return;
    };

    FRAMEBUFFER_CONSOLE.call_once(|| Arc::new(FramebufferConsole::new(fb.clone())));
}

impl AnyConsoleDevice for FramebufferConsole {
    fn send(&self, buf: &[u8]) {
        self.state.lock().send_buf(buf);
    }

    fn register_callback(&self, _: &'static ConsoleCallback) {
        // Unsupported, do nothing.
    }
}

impl FramebufferConsole {
    /// Creates a new framebuffer console.
    pub fn new(framebuffer: Arc<FrameBuffer>) -> Self {
        let bytes = alloc::vec![0u8; framebuffer.size()];
        Self {
            state: SpinLock::new(ConsoleState {
                enabled: true,
                x_pos: 0,
                y_pos: 0,
                fg_color: Pixel::WHITE,
                bg_color: Pixel::BLACK,
                bytes,
                backend: framebuffer,
            }),
        }
    }

    /// Returns whether the console is enabled.
    pub fn is_enabled(&self) -> bool {
        self.state.lock().enabled
    }

    /// Enables the console.
    pub fn enable(&self) {
        self.state.lock().enabled = true;
    }

    /// Disables the console.
    pub fn disable(&self) {
        self.state.lock().enabled = false;
    }

    /// Returns the current cursor position.
    pub fn cursor(&self) -> (usize, usize) {
        let state = self.state.lock();
        (state.x_pos, state.y_pos)
    }

    /// Sets the cursor position.
    pub fn set_cursor(&self, x: usize, y: usize) -> Result<()> {
        let mut state = self.state.lock();
        if x > state.backend.width() - FONT_WIDTH || y > state.backend.height() - FONT_HEIGHT {
            log::warn!("Invalid framebuffer cursor position: ({}, {})", x, y);
            return Err(Error::InvalidArgs);
        }
        state.x_pos = x;
        state.y_pos = y;
        Ok(())
    }

    /// Returns the foreground color.
    pub fn fg_color(&self) -> Pixel {
        self.state.lock().fg_color
    }

    /// Sets the foreground color.
    pub fn set_fg_color(&self, val: Pixel) {
        self.state.lock().fg_color = val;
    }

    /// Returns the background color.
    pub fn bg_color(&self) -> Pixel {
        self.state.lock().bg_color
    }

    /// Sets the background color.
    pub fn set_bg_color(&self, val: Pixel) {
        self.state.lock().bg_color = val;
    }
}

#[derive(Debug)]
struct ConsoleState {
    // FIXME: maybe we should drop the whole `ConsoleState` when it's disabled.
    enabled: bool,
    x_pos: usize,
    y_pos: usize,
    fg_color: Pixel,
    bg_color: Pixel,
    bytes: Vec<u8>,
    backend: Arc<FrameBuffer>,
}

impl ConsoleState {
    fn carriage_return(&mut self) {
        self.x_pos = 0;
    }

    fn newline(&mut self) {
        if self.y_pos >= self.backend.height() - FONT_HEIGHT {
            self.shift_lines_up();
        }
        self.y_pos += FONT_HEIGHT;
        self.x_pos = 0;
    }

    fn shift_lines_up(&mut self) {
        let offset = self.backend.calc_offset(0, FONT_HEIGHT).as_usize();
        self.bytes.copy_within(offset.., 0);
        self.bytes[self.backend.size() - offset..].fill(0);
        self.backend.write_bytes_at(0, &self.bytes).unwrap();
        self.y_pos -= FONT_HEIGHT;
    }

    /// Sends a single character to be drawn on the framebuffer.
    fn send_char(&mut self, c: char) {
        if c == '\n' {
            self.newline();
            return;
        } else if c == '\r' {
            self.carriage_return();
            return;
        }

        if self.x_pos + FONT_WIDTH > self.backend.width() {
            self.newline();
        }

        let rendered = font8x8::BASIC_FONTS
            .get(c)
            .expect("character not found in basic font");
        let fg_pixel = self.backend.render_pixel(self.fg_color);
        let bg_pixel = self.backend.render_pixel(self.bg_color);
        let mut offset = self.backend.calc_offset(self.x_pos, self.y_pos);
        for byte in rendered.iter() {
            for bit in 0..8 {
                let on = *byte & (1 << bit) != 0;
                let pixel = if on { fg_pixel } else { bg_pixel };

                // Cache the rendered pixel
                self.bytes[offset.as_usize()..offset.as_usize() + pixel.nbytes()]
                    .copy_from_slice(pixel.as_slice());
                // Write the pixel to the framebuffer
                self.backend.write_pixel_at(offset, pixel).unwrap();

                offset.x_add(1);
            }
            offset.x_add(-(FONT_WIDTH as isize));
            offset.y_add(1);
        }
        self.x_pos += FONT_WIDTH;
    }

    /// Sends a buffer of bytes to be drawn on the framebuffer.
    ///
    /// # Panics
    ///
    /// This method will panic if the buffer contains any characters
    /// other than Basic Latin characters (`U+0000` - `U+007F`).
    fn send_buf(&mut self, buf: &[u8]) {
        if !self.enabled {
            return;
        }

        // TODO: handle ANSI escape sequences.
        for &byte in buf.iter() {
            if byte != 0 {
                let char = char::from_u32(byte as u32).unwrap();
                self.send_char(char);
            }
        }
    }
}
