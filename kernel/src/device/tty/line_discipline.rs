// SPDX-License-Identifier: MPL-2.0

use alloc::format;

use super::termio::{KernelTermios, WinSize, CC_C_CHAR};
use crate::{
    prelude::*,
    process::signal::{
        constants::{SIGINT, SIGQUIT},
        sig_num::SigNum,
    },
    util::ring_buffer::RingBuffer,
};

// This implementation refers the implementation of linux
// https://elixir.bootlin.com/linux/latest/source/include/linux/tty_ldisc.h

const BUFFER_CAPACITY: usize = 4096;

pub struct LineDiscipline {
    /// Current line
    current_line: CurrentLine,
    /// The read buffer
    read_buffer: RingBuffer<u8>,
    /// Termios
    termios: KernelTermios,
    /// Windows size
    winsize: WinSize,
}

pub struct CurrentLine {
    buffer: RingBuffer<u8>,
}

impl Default for CurrentLine {
    fn default() -> Self {
        Self {
            buffer: RingBuffer::new(BUFFER_CAPACITY),
        }
    }
}

impl CurrentLine {
    /// Reads all bytes inside current line and clear current line
    pub fn drain(&mut self) -> Vec<u8> {
        let mut ret = vec![0u8; self.buffer.len()];
        self.buffer.pop_slice(ret.as_mut_slice()).unwrap();
        ret
    }

    pub fn push_char(&mut self, char: u8) {
        // What should we do if line is full?
        debug_assert!(!self.is_full());
        self.buffer.push_overwrite(char);
    }

    pub fn backspace(&mut self) {
        let _ = self.buffer.pop();
    }

    pub fn is_full(&self) -> bool {
        self.buffer.is_full()
    }

    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }
}

impl LineDiscipline {
    /// Creates a new line discipline.
    pub fn new() -> Self {
        Self {
            current_line: CurrentLine::default(),
            read_buffer: RingBuffer::new(BUFFER_CAPACITY),
            termios: KernelTermios::default(),
            winsize: WinSize::default(),
        }
    }

    /// Pushes a character to the line discipline.
    pub fn push_char<F1: FnMut(SigNum), F2: FnMut(&str)>(
        &mut self,
        ch: u8,
        mut signal_callback: F1,
        echo_callback: F2,
    ) {
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

        // Raw mode
        if !self.termios.is_canonical_mode() {
            self.read_buffer.push_overwrite(ch);
            return;
        }

        // Canonical mode

        if ch == *self.termios.get_special_char(CC_C_CHAR::VKILL) {
            // Erase current line
            self.current_line.drain();
        }

        if ch == *self.termios.get_special_char(CC_C_CHAR::VERASE) {
            // Type backspace
            self.current_line.backspace();
        }

        if is_line_terminator(ch, &self.termios) {
            // A new line is met. Move all bytes in `current_line` to `read_buffer`.
            self.current_line.push_char(ch);
            for line_ch in self.current_line.drain() {
                self.read_buffer.push_overwrite(line_ch);
            }
        }

        if is_printable_char(ch) {
            // Printable character
            self.current_line.push_char(ch);
        }
    }

    // TODO: respect output flags
    fn output_char<F: FnMut(&str)>(&self, ch: u8, mut echo_callback: F) {
        match ch {
            b'\n' => echo_callback("\n"),
            b'\r' => echo_callback("\r\n"),
            ch if ch == *self.termios.get_special_char(CC_C_CHAR::VERASE) => {
                // Write a space to overwrite the current character
                let backspace: &str = core::str::from_utf8(b"\x08 \x08").unwrap();
                echo_callback(backspace);
            }
            ch if is_printable_char(ch) => print!("{}", char::from(ch)),
            ch if is_ctrl_char(ch) && self.termios.contains_echo_ctl() => {
                let ctrl_char = format!("^{}", ctrl_char_to_printable(ch));
                echo_callback(&ctrl_char);
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
        let vmin = *self.termios.get_special_char(CC_C_CHAR::VMIN);
        let vtime = *self.termios.get_special_char(CC_C_CHAR::VTIME);

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

    pub fn termios(&self) -> &KernelTermios {
        &self.termios
    }

    pub fn set_termios(&mut self, termios: KernelTermios) {
        self.termios = termios;
    }

    pub fn drain_input(&mut self) {
        self.current_line.drain();
        self.read_buffer.clear();
    }

    pub fn buffer_len(&self) -> usize {
        self.read_buffer.len()
    }

    pub fn window_size(&self) -> WinSize {
        self.winsize
    }

    pub fn set_window_size(&mut self, winsize: WinSize) {
        self.winsize = winsize;
    }
}

fn is_line_terminator(ch: u8, termios: &KernelTermios) -> bool {
    if ch == b'\n'
        || ch == *termios.get_special_char(CC_C_CHAR::VEOF)
        || ch == *termios.get_special_char(CC_C_CHAR::VEOL)
    {
        return true;
    }

    if termios.contains_iexten() && ch == *termios.get_special_char(CC_C_CHAR::VEOL2) {
        return true;
    }

    false
}

fn is_eof(ch: u8, termios: &KernelTermios) -> bool {
    ch == *termios.get_special_char(CC_C_CHAR::VEOF)
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

fn char_to_signal(ch: u8, termios: &KernelTermios) -> Option<SigNum> {
    if !termios.is_canonical_mode() || !termios.contains_isig() {
        return None;
    }

    match ch {
        ch if ch == *termios.get_special_char(CC_C_CHAR::VINTR) => Some(SIGINT),
        ch if ch == *termios.get_special_char(CC_C_CHAR::VQUIT) => Some(SIGQUIT),
        _ => None,
    }
}

fn ctrl_char_to_printable(ch: u8) -> char {
    debug_assert!(is_ctrl_char(ch));
    char::from_u32((ch + b'A' - 1) as u32).unwrap()
}
