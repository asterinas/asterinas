// SPDX-License-Identifier: MPL-2.0

use alloc::{sync::Arc, vec::Vec};

use aster_console::{
    font::BitmapFont,
    mode::{ConsoleMode, KeyboardMode},
    AnyConsoleDevice, ConsoleCallback, ConsoleSetFontError,
};
use ostd::{
    mm::{HasSize, VmReader},
    sync::{LocalIrqDisabled, SpinLock},
};
use spin::Once;

use crate::{
    ansi_escape::{EscapeFsm, EscapeOp},
    FrameBuffer, Pixel, FRAMEBUFFER,
};

/// A text console rendered onto the framebuffer.
pub struct FramebufferConsole {
    callbacks: SpinLock<ConsoleCallbacks, LocalIrqDisabled>,
    inner: SpinLock<(ConsoleState, EscapeFsm), LocalIrqDisabled>,
}

pub const CONSOLE_NAME: &str = "Framebuffer-Console";

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
        let mut inner = self.inner.lock();
        let (state, esc_fsm) = &mut *inner;

        for byte in buf {
            if esc_fsm.eat(*byte, state) {
                // The character is part of an ANSI escape sequence.
                continue;
            }

            if *byte == 0 {
                // The character is a NUL character.
                continue;
            }

            state.send_char(*byte);
        }
    }

    fn register_callback(&self, callback: &'static ConsoleCallback) {
        self.callbacks.lock().callbacks.push(callback);
    }

    fn set_font(&self, font: BitmapFont) -> Result<(), ConsoleSetFontError> {
        self.inner.lock().0.set_font(font)
    }

    fn set_mode(&self, mode: ConsoleMode) -> bool {
        self.inner.lock().0.set_mode(mode);
        true
    }

    fn mode(&self) -> Option<ConsoleMode> {
        Some(self.inner.lock().0.mode())
    }

    fn set_keyboard_mode(&self, mode: KeyboardMode) -> bool {
        match mode {
            KeyboardMode::Xlate => self.callbacks.lock().is_input_enabled = true,
            KeyboardMode::Off => self.callbacks.lock().is_input_enabled = false,
            _ => return false,
        }
        true
    }

    fn keyboard_mode(&self) -> Option<KeyboardMode> {
        if self.callbacks.lock().is_input_enabled {
            Some(KeyboardMode::Xlate)
        } else {
            Some(KeyboardMode::Off)
        }
    }
}

impl FramebufferConsole {
    /// Creates a new framebuffer console.
    pub(self) fn new(framebuffer: Arc<FrameBuffer>) -> Self {
        let callbacks = ConsoleCallbacks {
            callbacks: Vec::new(),
            is_input_enabled: true,
        };

        let state = ConsoleState {
            x_pos: 0,
            y_pos: 0,
            fg_color: Pixel::WHITE,
            bg_color: Pixel::BLACK,
            font: BitmapFont::new_basic8x8(),
            is_output_enabled: true,

            bytes: alloc::vec![0u8; framebuffer.io_mem().size()],
            backend: framebuffer,
        };

        let esc_fsm = EscapeFsm::new();

        Self {
            callbacks: SpinLock::new(callbacks),
            inner: SpinLock::new((state, esc_fsm)),
        }
    }

    /// Triggers the registered input callbacks with the given data.
    pub(crate) fn trigger_input_callbacks(&self, bytes: &[u8]) {
        let callbacks = self.callbacks.lock();
        if !callbacks.is_input_enabled {
            return;
        }

        let reader = VmReader::from(bytes);
        for callback in callbacks.callbacks.iter() {
            callback(reader.clone());
        }
    }
}

impl core::fmt::Debug for FramebufferConsole {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FramebufferConsole").finish_non_exhaustive()
    }
}

struct ConsoleCallbacks {
    callbacks: Vec<&'static ConsoleCallback>,
    /// Whether the input characters will be handled by the callbacks.
    is_input_enabled: bool,
}

#[derive(Debug)]
struct ConsoleState {
    x_pos: usize,
    y_pos: usize,
    fg_color: Pixel,
    bg_color: Pixel,
    font: BitmapFont,
    /// Whether the output characters will be drawn in the framebuffer.
    is_output_enabled: bool,

    bytes: Vec<u8>,
    backend: Arc<FrameBuffer>,
}

impl ConsoleState {
    /// Sends a single character to be drawn on the framebuffer.
    pub(self) fn send_char(&mut self, ch: u8) {
        if ch == b'\n' {
            self.newline();
            return;
        } else if ch == b'\r' {
            self.carriage_return();
            return;
        } else if ch == b'\x08' {
            self.backspace();
            return;
        }

        if self.x_pos > self.backend.width() - self.font.width() {
            self.newline();
        }

        self.draw_char(ch);

        self.x_pos += self.font.width();
    }

    fn newline(&mut self) {
        self.y_pos += self.font.height();
        self.x_pos = 0;

        if self.y_pos > self.backend.height() - self.font.height() {
            self.shift_lines_up();
        }
    }

    fn shift_lines_up(&mut self) {
        let offset = self.backend.calc_offset(0, self.font.height()).as_usize();
        self.bytes.copy_within(offset.., 0);
        self.bytes[self.backend.io_mem().size() - offset..].fill(0);

        if self.is_output_enabled {
            self.backend.write_bytes_at(0, &self.bytes).unwrap();
        }

        self.y_pos -= self.font.height();
    }

    fn carriage_return(&mut self) {
        self.x_pos = 0;
    }

    fn backspace(&mut self) {
        if self.x_pos < self.font.width() {
            // TODO: What should we do if we're at the beginning of the line?
            return;
        }

        self.x_pos -= self.font.width();
        self.draw_char(b' ');
    }

    fn draw_char(&mut self, ch: u8) {
        let Some(font_ch) = self.font.char(ch) else {
            return;
        };

        let fg_pixel = self.backend.render_pixel(self.fg_color);
        let bg_pixel = self.backend.render_pixel(self.bg_color);

        let pixel_size = fg_pixel.nbytes();

        let mut offset = self.backend.calc_offset(self.x_pos, self.y_pos);

        for row in font_ch.rows() {
            let off_st = offset.as_usize();
            let off_ed = off_st + pixel_size * self.font.width();
            let render_buf = &mut self.bytes[off_st..off_ed];

            // Write pixels to the console buffer.
            let chunks = render_buf.chunks_exact_mut(pixel_size);
            for (chunk, is_fg) in chunks.zip(row.bits()) {
                let pixel = if is_fg { fg_pixel } else { bg_pixel };
                chunk.copy_from_slice(pixel.as_slice());
            }

            // Write pixels to the framebuffer.
            if self.is_output_enabled {
                self.backend.write_bytes_at(off_st, render_buf).unwrap();
            }

            offset.y_add(1);
        }
    }

    /// Sets the font for the framebuffer console.
    pub(self) fn set_font(&mut self, font: BitmapFont) -> Result<(), ConsoleSetFontError> {
        // Note that the font height cannot exceed the half the height of the framebuffer.
        // Otherwise, `shift_lines_up` will underflow `x_pos`.
        if font.width() > self.backend.width() || font.height() > self.backend.height() / 2 {
            return Err(ConsoleSetFontError::InvalidFont);
        }

        self.font = font;

        if self.y_pos > self.backend.height() - self.font.height() {
            self.shift_lines_up();
        }

        Ok(())
    }

    /// Sets the console mode (text or graphics).
    pub(self) fn set_mode(&mut self, mode: ConsoleMode) {
        if mode == ConsoleMode::Graphics {
            self.is_output_enabled = false;
            return;
        }

        if self.is_output_enabled {
            return;
        }

        // We're switching from the graphics mode back to the text mode. The characters need to be
        // redrawn in the framebuffer.
        self.is_output_enabled = true;
        self.backend.write_bytes_at(0, &self.bytes).unwrap();
    }

    /// Gets the current console mode.
    pub(self) fn mode(&self) -> ConsoleMode {
        if self.is_output_enabled {
            ConsoleMode::Text
        } else {
            ConsoleMode::Graphics
        }
    }
}

impl EscapeOp for ConsoleState {
    fn set_cursor(&mut self, x: usize, y: usize) {
        let max_x = self.backend.width() / self.font.width() - 1;
        let max_y = self.backend.height() / self.font.height() - 1;

        // Note that if the Y (or X) position is too large, the cursor will move to the last line
        // (or the line end).
        self.x_pos = x.min(max_x) * self.font.width();
        self.y_pos = y.min(max_y) * self.font.height();
    }

    fn set_fg_color(&mut self, val: Pixel) {
        self.fg_color = val;
    }

    fn set_bg_color(&mut self, val: Pixel) {
        self.bg_color = val;
    }
}
