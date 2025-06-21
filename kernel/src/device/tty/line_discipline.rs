// SPDX-License-Identifier: MPL-2.0

use ostd::const_assert;

use super::{
    termio::{CCtrlCharId, CTermios, CWinSize},
    PushCharError,
};
use crate::{
    prelude::*,
    process::signal::{
        constants::{SIGINT, SIGQUIT},
        sig_num::SigNum,
    },
    util::ring_buffer::RingBuffer,
};

// This implementation references the implementation of Linux:
// <https://elixir.bootlin.com/linux/latest/source/include/linux/tty_ldisc.h>

const LINE_CAPACITY: usize = 4095;
const BUFFER_CAPACITY: usize = 8192;

// `LINE_CAPACITY` must be less than `BUFFER_CAPACITY`. Otherwise, `write()` can be blocked
// indefinitely if both the current line and the buffer are full, so even the line terminator won't
// be accepted.
const_assert!(LINE_CAPACITY < BUFFER_CAPACITY);

pub struct LineDiscipline {
    /// Current line
    current_line: CurrentLine,
    /// Read buffer
    read_buffer: RingBuffer<u8>,
    /// Termios
    termios: CTermios,
    /// Window size
    winsize: CWinSize,
}

struct CurrentLine {
    buffer: Box<[u8]>,
    len: usize,
}

impl Default for CurrentLine {
    fn default() -> Self {
        Self {
            buffer: vec![0; LINE_CAPACITY].into_boxed_slice(),
            len: 0,
        }
    }
}

impl CurrentLine {
    /// Pushes a character to the current line.
    fn push_char(&mut self, ch: u8) {
        // If the line is full, the character will be ignored, but other actions such as echoing
        // and signaling will work as normal. This will never block the caller, even if the input
        // comes from the pseduoterminal master.
        if self.len == self.buffer.len() {
            return;
        }

        self.buffer[self.len] = ch;
        self.len += 1;
    }

    /// Clears the current line and returns the bytes in it.
    fn drain(&mut self) -> &[u8] {
        let chs = &self.buffer[..self.len];
        self.len = 0;
        chs
    }

    /// Removes the last character, if it is present.
    fn backspace(&mut self) {
        self.len = self.len.saturating_sub(1);
    }

    /// Returns the number of characters in the current line.
    fn len(&self) -> usize {
        self.len
    }
}

impl LineDiscipline {
    /// Creates a new line discipline.
    pub fn new() -> Self {
        Self {
            current_line: CurrentLine::default(),
            read_buffer: RingBuffer::new(BUFFER_CAPACITY),
            termios: CTermios::default(),
            winsize: CWinSize::default(),
        }
    }

    /// Pushes a character to the line discipline.
    pub fn push_char<F1: FnMut(SigNum), F2: FnMut(&[u8])>(
        &mut self,
        ch: u8,
        mut signal_callback: F1,
        echo_callback: F2,
    ) -> core::result::Result<(), PushCharError> {
        let ch = if self.termios.contains_icrnl() && ch == b'\r' {
            b'\n'
        } else {
            ch
        };

        if let Some(signum) = char_to_signal(ch, &self.termios) {
            signal_callback(signum);
            // CBREAK mode may require the character to be echoed, so just go ahead.
        }

        // Typically, a TTY in raw mode does not echo. But the TTY can also be in a CBREAK mode,
        // with ICANON closed and ECHO opened.
        if self.termios.contain_echo() {
            self.output_char(ch, echo_callback);
        }

        if self.is_full() {
            // If the buffer is full, we should not push the character into the buffer. The caller
            // can silently ignore the error (if the input comes from the keyboard) or block the
            // user space (if the input comes from the pseduoterminal master).
            return Err(PushCharError);
        }

        // Raw mode
        if !self.termios.is_canonical_mode() {
            // Note that `unwrap()` below won't fail because we checked `is_full()` above.
            self.read_buffer.push(ch).unwrap();
            return Ok(());
        }

        // Canonical mode

        if ch == self.termios.special_char(CCtrlCharId::VKILL) {
            // Erase current line
            self.current_line.drain();
        }

        if ch == self.termios.special_char(CCtrlCharId::VERASE) {
            // Type backspace
            self.current_line.backspace();
        }

        if is_line_terminator(ch, &self.termios) {
            // A new line is met. Move all bytes in `current_line` to `read_buffer`.
            // Note that `unwrap()` below won't fail because we checked `is_full()` above.
            for line_ch in self.current_line.drain() {
                self.read_buffer.push(*line_ch).unwrap();
            }
            self.read_buffer.push(ch).unwrap();
        }

        if is_printable_char(ch) {
            // Printable character
            self.current_line.push_char(ch);
        }

        Ok(())
    }

    // TODO: respect output flags
    fn output_char<F: FnMut(&[u8])>(&self, ch: u8, mut echo_callback: F) {
        match ch {
            b'\n' => echo_callback(b"\n"),
            b'\r' => echo_callback(b"\r\n"),
            ch if ch == self.termios.special_char(CCtrlCharId::VERASE) => {
                // The driver should erase the current character
                echo_callback(b"\x08");
            }
            ch if is_printable_char(ch) => echo_callback(&[ch]),
            ch if is_ctrl_char(ch) && self.termios.contains_echo_ctl() => {
                echo_callback(&[b'^', ctrl_char_to_printable(ch)]);
            }
            _ => {}
        }
    }

    /// Reads bytes from `self` to `dst`, returning the actual bytes read.
    ///
    /// # Errors
    ///
    /// If no bytes are available or the available bytes are fewer than `min(dst.len(), vmin)`,
    /// this method returns [`Errno::EAGAIN`].
    pub fn try_read(&mut self, dst: &mut [u8]) -> Result<usize> {
        let vmin = self.termios.special_char(CCtrlCharId::VMIN);
        let vtime = self.termios.special_char(CCtrlCharId::VTIME);

        if vtime != 0 {
            warn!("non-zero VTIME is not supported");
        }

        // If `vmin` is zero or `dst` is empty, the following condition will always be false. This
        // is correct, as the expected behavior is to never block or return `EAGAIN`.
        if self.buffer_len() < dst.len().min(vmin as _) {
            return_errno_with_message!(
                Errno::EAGAIN,
                "the characters in the buffer are not enough"
            );
        }

        for (i, dst_i) in dst.iter_mut().enumerate() {
            let Some(ch) = self.read_buffer.pop() else {
                // No more characters
                return Ok(i);
            };

            if self.termios.is_canonical_mode() && is_eof(ch, &self.termios) {
                // This allows the userspace program to see `Ok(0)`
                return Ok(i);
            }

            *dst_i = ch;

            if self.termios.is_canonical_mode() && is_line_terminator(ch, &self.termios) {
                // Read until line terminators in canonical mode
                return Ok(i + 1);
            }
        }

        Ok(dst.len())
    }

    pub fn drain_input(&mut self) {
        self.current_line.drain();
        self.read_buffer.clear();
    }

    pub fn buffer_len(&self) -> usize {
        self.read_buffer.len()
    }

    pub fn is_full(&self) -> bool {
        self.read_buffer.len() + self.current_line.len() >= self.read_buffer.capacity()
    }

    pub fn termios(&self) -> &CTermios {
        &self.termios
    }

    pub fn set_termios(&mut self, termios: CTermios) {
        self.termios = termios;
    }

    pub fn window_size(&self) -> CWinSize {
        self.winsize
    }

    pub fn set_window_size(&mut self, winsize: CWinSize) {
        self.winsize = winsize;
    }
}

fn is_line_terminator(ch: u8, termios: &CTermios) -> bool {
    if ch == b'\n'
        || ch == termios.special_char(CCtrlCharId::VEOF)
        || ch == termios.special_char(CCtrlCharId::VEOL)
    {
        return true;
    }

    if termios.contains_iexten() && ch == termios.special_char(CCtrlCharId::VEOL2) {
        return true;
    }

    false
}

fn is_eof(ch: u8, termios: &CTermios) -> bool {
    ch == termios.special_char(CCtrlCharId::VEOF)
}

fn is_printable_char(ch: u8) -> bool {
    (0x20..0x7f).contains(&ch)
}

fn is_ctrl_char(ch: u8) -> bool {
    if ch == b'\r' || ch == b'\n' {
        return false;
    }

    (0..0x20).contains(&ch)
}

fn char_to_signal(ch: u8, termios: &CTermios) -> Option<SigNum> {
    if !termios.is_canonical_mode() || !termios.contains_isig() {
        return None;
    }

    match ch {
        ch if ch == termios.special_char(CCtrlCharId::VINTR) => Some(SIGINT),
        ch if ch == termios.special_char(CCtrlCharId::VQUIT) => Some(SIGQUIT),
        _ => None,
    }
}

fn ctrl_char_to_printable(ch: u8) -> u8 {
    debug_assert!(is_ctrl_char(ch));
    ch + b'A' - 1
}
