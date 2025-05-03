// SPDX-License-Identifier: MPL-2.0

use alloc::format;

use ostd::{sync::LocalIrqDisabled, trap::disable_local};

use super::termio::{KernelTermios, WinSize, CC_C_CHAR};
use crate::{
    events::IoEvents,
    prelude::*,
    process::signal::{
        constants::{SIGINT, SIGQUIT},
        sig_num::SigNum,
        PollHandle, Pollable, Pollee,
    },
    util::ring_buffer::RingBuffer,
};

// This implementation refers the implementation of linux
// https://elixir.bootlin.com/linux/latest/source/include/linux/tty_ldisc.h

const BUFFER_CAPACITY: usize = 4096;

// Lock ordering to prevent deadlock (circular dependencies):
// 1. `termios`
// 2. `current_line`
// 3. `read_buffer`
// 4. `work_item_para`
pub struct LineDiscipline {
    /// Current line
    current_line: SpinLock<CurrentLine, LocalIrqDisabled>,
    /// The read buffer
    read_buffer: SpinLock<RingBuffer<u8>, LocalIrqDisabled>,
    /// Termios
    termios: SpinLock<KernelTermios, LocalIrqDisabled>,
    /// Windows size
    winsize: SpinLock<WinSize, LocalIrqDisabled>,
    /// Pollee
    pollee: Pollee,
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

impl Pollable for LineDiscipline {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        self.pollee
            .poll_with(mask, poller, || self.check_io_events())
    }
}

impl LineDiscipline {
    /// Creates a new line discipline
    pub fn new() -> Self {
        Self {
            current_line: SpinLock::new(CurrentLine::default()),
            read_buffer: SpinLock::new(RingBuffer::new(BUFFER_CAPACITY)),
            termios: SpinLock::new(KernelTermios::default()),
            winsize: SpinLock::new(WinSize::default()),
            pollee: Pollee::new(),
        }
    }

    /// Pushes a char to the line discipline
    pub fn push_char<F1: FnMut(SigNum), F2: FnMut(&str)>(
        &self,
        ch: u8,
        mut signal_callback: F1,
        echo_callback: F2,
    ) {
        let termios = self.termios.lock();

        let ch = if termios.contains_icrnl() && ch == b'\r' {
            b'\n'
        } else {
            ch
        };

        if let Some(signum) = char_to_signal(ch, &termios) {
            signal_callback(signum);
            // CBREAK mode may require the character to be outputted, so just go ahead.
        }

        // Typically, a tty in raw mode does not echo. But the tty can also be in a CBREAK mode,
        // with ICANON closed and ECHO opened.
        if termios.contain_echo() {
            self.output_char(ch, &termios, echo_callback);
        }

        // Raw mode
        if !termios.is_canonical_mode() {
            self.read_buffer.lock().push_overwrite(ch);
            self.pollee.notify(IoEvents::IN);
            return;
        }

        // Canonical mode

        if ch == *termios.get_special_char(CC_C_CHAR::VKILL) {
            // Erase current line
            self.current_line.lock().drain();
        }

        if ch == *termios.get_special_char(CC_C_CHAR::VERASE) {
            // Type backspace
            let mut current_line = self.current_line.lock();
            if !current_line.is_empty() {
                current_line.backspace();
            }
        }

        if is_line_terminator(ch, &termios) {
            // If a new line is met, all bytes in current_line will be moved to read_buffer
            let mut current_line = self.current_line.lock();
            current_line.push_char(ch);
            let current_line_chars = current_line.drain();
            for char in current_line_chars {
                self.read_buffer.lock().push_overwrite(char);
                self.pollee.notify(IoEvents::IN);
            }
        }

        if is_printable_char(ch) {
            // Printable character
            self.current_line.lock().push_char(ch);
        }
    }

    fn check_io_events(&self) -> IoEvents {
        let buffer = self.read_buffer.lock();

        if !buffer.is_empty() {
            IoEvents::IN
        } else {
            IoEvents::empty()
        }
    }

    // TODO: respect output flags
    fn output_char<F: FnMut(&str)>(&self, ch: u8, termios: &KernelTermios, mut echo_callback: F) {
        match ch {
            b'\n' => echo_callback("\n"),
            b'\r' => echo_callback("\r\n"),
            ch if ch == *termios.get_special_char(CC_C_CHAR::VERASE) => {
                // write a space to overwrite current character
                let backspace: &str = core::str::from_utf8(b"\x08 \x08").unwrap();
                echo_callback(backspace);
            }
            ch if is_printable_char(ch) => print!("{}", char::from(ch)),
            ch if is_ctrl_char(ch) && termios.contains_echo_ctl() => {
                let ctrl_char = format!("^{}", ctrl_char_to_printable(ch));
                echo_callback(&ctrl_char);
            }
            _ => {}
        }
    }

    pub fn read(&self, buf: &mut [u8]) -> Result<usize> {
        self.wait_events(IoEvents::IN, None, || self.try_read(buf))
    }

    /// Reads all bytes buffered to `dst`.
    ///
    /// This method returns the actual read length.
    fn try_read(&self, dst: &mut [u8]) -> Result<usize> {
        let (vmin, vtime) = {
            let termios = self.termios.lock();
            let vmin = *termios.get_special_char(CC_C_CHAR::VMIN);
            let vtime = *termios.get_special_char(CC_C_CHAR::VTIME);
            (vmin, vtime)
        };
        let read_len = {
            if vmin == 0 && vtime == 0 {
                // poll read
                self.poll_read(dst)
            } else if vmin > 0 && vtime == 0 {
                // block read
                self.block_read(dst, vmin)?
            } else if vmin == 0 && vtime > 0 {
                todo!()
            } else if vmin > 0 && vtime > 0 {
                todo!()
            } else {
                unreachable!()
            }
        };
        self.pollee.invalidate();
        Ok(read_len)
    }

    /// Reads bytes from `self` to `dst`, returning the actual bytes read.
    ///
    /// If no bytes are available, this method returns 0 immediately.
    fn poll_read(&self, dst: &mut [u8]) -> usize {
        let termios = self.termios.lock();
        let mut buffer = self.read_buffer.lock();
        let len = buffer.len();
        let max_read_len = len.min(dst.len());
        if max_read_len == 0 {
            return 0;
        }
        let mut read_len = 0;
        for dst_i in dst.iter_mut().take(max_read_len) {
            if let Some(next_char) = buffer.pop() {
                if termios.is_canonical_mode() {
                    // canonical mode, read until meet new line
                    if is_line_terminator(next_char, &termios) {
                        // The eof should not be read
                        if !is_eof(next_char, &termios) {
                            *dst_i = next_char;
                            read_len += 1;
                        }
                        break;
                    } else {
                        *dst_i = next_char;
                        read_len += 1;
                    }
                } else {
                    // raw mode
                    // FIXME: avoid additional bound check
                    *dst_i = next_char;
                    read_len += 1;
                }
            } else {
                break;
            }
        }

        read_len
    }

    /// Reads bytes from `self` into `dst`,
    /// returning the actual number of bytes read.
    ///
    /// # Errors
    ///
    /// If the available bytes are fewer than `min(dst.len(), vmin)`,
    /// this method returns [`Errno::EAGAIN`].
    pub fn block_read(&self, dst: &mut [u8], vmin: u8) -> Result<usize> {
        let _guard = disable_local();
        let buffer_len = self.read_buffer.lock().len();
        if buffer_len >= dst.len() {
            return Ok(self.poll_read(dst));
        }
        if buffer_len < vmin as usize {
            return_errno!(Errno::EAGAIN);
        }
        Ok(self.poll_read(&mut dst[..buffer_len]))
    }

    /// Returns whether there is buffered data
    pub fn is_empty(&self) -> bool {
        self.read_buffer.lock().len() == 0
    }

    pub fn termios(&self) -> KernelTermios {
        *self.termios.lock()
    }

    pub fn set_termios(&self, termios: KernelTermios) {
        *self.termios.lock() = termios;
    }

    pub fn drain_input(&self) {
        self.current_line.lock().drain();
        self.read_buffer.lock().clear();
        self.pollee.invalidate();
    }

    pub fn buffer_len(&self) -> usize {
        self.read_buffer.lock().len()
    }

    pub fn window_size(&self) -> WinSize {
        *self.winsize.lock()
    }

    pub fn set_window_size(&self, winsize: WinSize) {
        *self.winsize.lock() = winsize;
    }
}

fn is_line_terminator(item: u8, termios: &KernelTermios) -> bool {
    if item == b'\n'
        || item == *termios.get_special_char(CC_C_CHAR::VEOF)
        || item == *termios.get_special_char(CC_C_CHAR::VEOL)
    {
        return true;
    }

    if termios.contains_iexten() && item == *termios.get_special_char(CC_C_CHAR::VEOL2) {
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
