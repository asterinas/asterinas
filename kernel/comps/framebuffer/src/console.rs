// SPDX-License-Identifier: MPL-2.0

use alloc::{sync::Arc, vec::Vec};

use aster_console::{
    AnyConsoleDevice, ConsoleCallback, ConsoleSetFontError,
    font::BitmapFont,
    mode::{ConsoleMode, KeyboardMode, KeyboardModeFlags},
};
use ostd::{
    mm::{HasSize, VmReader},
    sync::{LocalIrqDisabled, SpinLock, SpinLockGuard},
};
use spin::Once;

use crate::{
    FRAMEBUFFER, FrameBuffer, Pixel,
    ansi_escape::{EraseInDisplay, EscapeFsm, EscapeOp},
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
        ostd::warn!("Framebuffer not initialized");
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
        let mut callbacks = self.callbacks.lock();
        match mode {
            // TODO: Add support for Raw mode.
            KeyboardMode::Raw => false,
            KeyboardMode::Xlate
            | KeyboardMode::MediumRaw
            | KeyboardMode::Unicode
            | KeyboardMode::Off => {
                callbacks.keyboard_mode = mode;
                true
            }
        }
    }

    fn keyboard_mode(&self) -> Option<KeyboardMode> {
        Some(self.callbacks.lock().keyboard_mode())
    }
}

impl FramebufferConsole {
    /// Creates a new framebuffer console.
    pub(self) fn new(framebuffer: Arc<FrameBuffer>) -> Self {
        let callbacks = ConsoleCallbacks {
            callbacks: Vec::new(),
            keyboard_mode: KeyboardMode::Unicode,
            // Linux default: REPEAT | META
            // Reference: <https://elixir.bootlin.com/linux/v6.17.4/source/drivers/tty/vt/keyboard.c#L56>
            keyboard_mode_flags: KeyboardModeFlags::REPEAT | KeyboardModeFlags::META,
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

    /// Locks the console callbacks.
    pub fn lock_callbacks(&self) -> SpinLockGuard<'_, ConsoleCallbacks, LocalIrqDisabled> {
        self.callbacks.lock()
    }
}

impl core::fmt::Debug for FramebufferConsole {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FramebufferConsole").finish_non_exhaustive()
    }
}

pub struct ConsoleCallbacks {
    callbacks: Vec<&'static ConsoleCallback>,
    keyboard_mode: KeyboardMode,
    keyboard_mode_flags: KeyboardModeFlags,
}

impl ConsoleCallbacks {
    /// Triggers the registered input callbacks with the given data.
    pub fn trigger_callbacks(&self, bytes: &[u8]) {
        let reader = VmReader::from(bytes);
        for callback in self.callbacks.iter() {
            callback(reader.clone());
        }
    }

    /// Returns the keyboard mode.
    pub fn keyboard_mode(&self) -> KeyboardMode {
        self.keyboard_mode
    }

    /// Returns the keyboard mode flags.
    pub fn keyboard_mode_flags(&self) -> KeyboardModeFlags {
        self.keyboard_mode_flags
    }
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

    /// Fills a rectangular pixel region `[x0, x1) × [y0, y1)` with the given color.
    ///
    /// The caller must pass arguments that form a valid rectangle within the framebuffer, i.e.,
    /// `x0 ≤ x1 ≤ w` and `y0 ≤ y1 ≤ h`, where `w` and `h` are the framebuffer width and height.
    fn fill_rect_pixels(&mut self, x0: usize, y0: usize, x1: usize, y1: usize, color: Pixel) {
        debug_assert!(x1 <= self.backend.width());
        debug_assert!(y1 <= self.backend.height());

        let rendered_pixel = self.backend.render_pixel(color);
        let rendered_pixel_size = rendered_pixel.nbytes();
        let row_bytes = (x1 - x0) * rendered_pixel_size;

        for y in y0..y1 {
            let off = self.backend.calc_offset(x0, y).as_usize();
            let buf = &mut self.bytes[off..off + row_bytes];

            // Write pixels to the console buffer.
            for chunk in buf.chunks_exact_mut(rendered_pixel_size) {
                chunk.copy_from_slice(rendered_pixel.as_slice());
            }

            // Write pixels to the framebuffer.
            if self.is_output_enabled {
                self.backend.write_bytes_at(off, buf).unwrap();
            }
        }
    }

    /// Calculates the pixel coordinates for the cursor cell.
    fn cursor_cell_rect(&self) -> (usize, usize, usize, usize) {
        let cx0 = self.x_pos;
        let cx1 = cx0 + self.font.width();
        // `min` is necessary if we are at the end of the line.
        let cx1 = cx1.min(self.backend.width());

        let cy0 = self.y_pos;
        let cy1 = cy0 + self.font.height();
        // This will always hold because we will scroll if it is no longer true after a new line or
        // font change.
        debug_assert!(cy1 <= self.backend.height());

        (cx0, cy0, cx1, cy1)
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

    fn erase_in_display(&mut self, mode: EraseInDisplay) {
        let bg = self.bg_color;
        let w = self.backend.width();
        let h = self.backend.height();

        let (cx0, cy0, cx1, cy1) = self.cursor_cell_rect();

        match mode {
            EraseInDisplay::CursorToEnd => {
                // Clear from the cursor to the end of the line, within the cursor row.
                self.fill_rect_pixels(cx0, cy0, w, cy1, bg);

                // Clear all rows below the cursor row.
                if cy1 < h {
                    self.fill_rect_pixels(0, cy1, w, h, bg);
                }
            }
            EraseInDisplay::CursorToBeginning => {
                // Clear all rows above the cursor row.
                if cy0 > 0 {
                    self.fill_rect_pixels(0, 0, w, cy0, bg);
                }

                // Clear from the start of the line to the cursor, within the cursor row.
                self.fill_rect_pixels(0, cy0, cx1, cy1, bg);
            }
            EraseInDisplay::EntireScreen | EraseInDisplay::EntireScreenAndScrollback => {
                // Clear the entire screen.
                self.fill_rect_pixels(0, 0, w, h, bg);
            }
        }
    }
}
