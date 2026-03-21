// SPDX-License-Identifier: MPL-2.0

use alloc::{sync::Arc, vec::Vec};

use aster_console::{ConsoleSetFontError, font::BitmapFont, mode::ConsoleMode};
use ostd::mm::HasSize;

use crate::{
    FrameBuffer, Pixel,
    ansi_escape::{EraseInDisplay, EscapeFsm, EscapeOp},
};

/// A text console rendered onto the framebuffer.
pub struct FramebufferConsole {
    state: ConsoleState,
    escape_fsm: EscapeFsm,
}

impl FramebufferConsole {
    /// Creates a new framebuffer console.
    pub fn new(framebuffer: Arc<FrameBuffer>) -> Self {
        let state = ConsoleState::new(framebuffer);
        let escape_fsm = EscapeFsm::new();

        Self { state, escape_fsm }
    }

    /// Returns the current console mode.
    pub fn mode(&self) -> ConsoleMode {
        self.state.mode
    }

    /// Sets the console mode.
    pub fn set_mode(&mut self, mode: ConsoleMode) {
        let old_mode = self.state.mode;
        self.state.mode = mode;
        if old_mode == ConsoleMode::Graphics {
            self.state.flush_fullscreen();
        }
    }

    /// Sets the font for the framebuffer console.
    pub fn set_font(&mut self, font: BitmapFont) -> Result<(), ConsoleSetFontError> {
        self.state.set_font(font)
    }

    /// Activates the console.
    ///
    /// If the console mode is [ConsoleMode::Text], the console will flush the entire
    /// console buffer to the framebuffer.
    pub fn activate(&mut self) {
        self.state.is_active = true;
        if self.state.mode == ConsoleMode::Text {
            self.state.flush_fullscreen();
        }
    }

    /// Deactivates the console, preventing it from rendering to the framebuffer
    /// even if text is sent to it.
    pub fn deactivate(&mut self) {
        self.state.is_active = false;
    }

    pub fn send(&mut self, buf: &[u8]) {
        for byte in buf {
            if self.escape_fsm.eat(*byte, &mut self.state) {
                // The character is part of an ANSI escape sequence.
                continue;
            }

            if *byte == 0 {
                // The character is a NUL character.
                continue;
            }

            self.state.send_char(*byte);
        }
    }
}

impl core::fmt::Debug for FramebufferConsole {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FramebufferConsole").finish_non_exhaustive()
    }
}

#[derive(Debug)]
struct ConsoleState {
    x_pos: usize,
    y_pos: usize,
    fg_color: Pixel,
    bg_color: Pixel,
    font: BitmapFont,
    /// Whether the console is active.
    is_active: bool,
    mode: ConsoleMode,

    bytes: Vec<u8>,
    backend: Arc<FrameBuffer>,
}

impl ConsoleState {
    fn new(backend: Arc<FrameBuffer>) -> Self {
        let buffer_size = backend.io_mem().size();
        Self {
            x_pos: 0,
            y_pos: 0,
            fg_color: Pixel::WHITE,
            bg_color: Pixel::BLACK,
            font: BitmapFont::new_basic8x8(),
            is_active: false,
            mode: ConsoleMode::Text,
            bytes: alloc::vec![0; buffer_size],
            backend,
        }
    }

    /// Flushes the entire console buffer to the framebuffer.
    fn flush_fullscreen(&mut self) {
        if self.is_active && self.mode == ConsoleMode::Text {
            self.backend.write_bytes_at(0, &self.bytes).unwrap();
        }
    }

    /// Sends a single character to be drawn on the framebuffer.
    fn send_char(&mut self, ch: u8) {
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

        if self.is_active && self.mode == ConsoleMode::Text {
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
            if self.is_active && self.mode == ConsoleMode::Text {
                self.backend.write_bytes_at(off_st, render_buf).unwrap();
            }

            offset.y_add(1);
        }
    }

    /// Sets the font for the framebuffer console.
    fn set_font(&mut self, font: BitmapFont) -> Result<(), ConsoleSetFontError> {
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
            if self.is_active && self.mode == ConsoleMode::Text {
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
