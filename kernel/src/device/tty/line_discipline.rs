// SPDX-License-Identifier: MPL-2.0

#![allow(unused_variables)]

use alloc::format;

use ostd::{
    sync::LocalIrqDisabled,
    trap::{disable_local, in_interrupt_context},
};

use super::termio::{KernelTermios, WinSize, CC_C_CHAR};
use crate::{
    events::IoEvents,
    prelude::*,
    process::signal::{
        constants::{SIGINT, SIGQUIT},
        signals::kernel::KernelSignal,
        Pollee, Poller,
    },
    thread::work_queue::{submit_work_item, work_item::WorkItem, WorkPriority},
    util::ring_buffer::RingBuffer,
};

// This implementation refers the implementation of linux
// https://elixir.bootlin.com/linux/latest/source/include/linux/tty_ldisc.h

const BUFFER_CAPACITY: usize = 4096;

pub type LdiscSignalSender = Arc<dyn Fn(KernelSignal) + Send + Sync + 'static>;

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
    /// Used to send signal for foreground processes, when some char comes.
    send_signal: LdiscSignalSender,
    /// Work item
    work_item: Arc<WorkItem>,
    /// Parameters used by a work item.
    work_item_para: Arc<SpinLock<LineDisciplineWorkPara, LocalIrqDisabled>>,
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
    /// Creates a new line discipline
    pub fn new(send_signal: LdiscSignalSender) -> Arc<Self> {
        Arc::new_cyclic(move |line_ref: &Weak<LineDiscipline>| {
            let line_discipline = line_ref.clone();
            let work_item = Arc::new(WorkItem::new(Box::new(move || {
                if let Some(line_discipline) = line_discipline.upgrade() {
                    line_discipline.send_signal_after();
                }
            })));
            Self {
                current_line: SpinLock::new(CurrentLine::default()),
                read_buffer: SpinLock::new(RingBuffer::new(BUFFER_CAPACITY)),
                termios: SpinLock::new(KernelTermios::default()),
                winsize: SpinLock::new(WinSize::default()),
                pollee: Pollee::new(IoEvents::empty()),
                send_signal,
                work_item,
                work_item_para: Arc::new(SpinLock::new(LineDisciplineWorkPara::new())),
            }
        })
    }

    /// Pushes a char to the line discipline
    pub fn push_char<F2: FnMut(&str)>(&self, ch: u8, echo_callback: F2) {
        self.termios.lock_with(|termios| {
            let ch = if termios.contains_icrnl() && ch == b'\r' {
                b'\n'
            } else {
                ch
            };

            if self.may_send_signal(termios, ch) {
                submit_work_item(self.work_item.clone(), WorkPriority::High);
                // CBREAK mode may require the character to be outputted, so just go ahead.
            }

            // Typically, a tty in raw mode does not echo. But the tty can also be in a CBREAK mode,
            // with ICANON closed and ECHO opened.
            if termios.contain_echo() {
                self.output_char(ch, termios, echo_callback);
            }

            // Raw mode
            if !termios.is_canonical_mode() {
                self.read_buffer.lock_with(|rb| rb.push_overwrite(ch));
                self.update_readable_state();
                return;
            }

            // Canonical mode

            if ch == *termios.get_special_char(CC_C_CHAR::VKILL) {
                // Erase current line
                self.current_line.lock_with(|l| l.drain());
            }

            if ch == *termios.get_special_char(CC_C_CHAR::VERASE) {
                // Type backspace
                self.current_line.lock_with(|current_line| {
                    if !current_line.is_empty() {
                        current_line.backspace();
                    }
                });
            }

            if is_line_terminator(ch, termios) {
                // If a new line is met, all bytes in current_line will be moved to read_buffer
                self.current_line.lock_with(|current_line| {
                    current_line.push_char(ch);
                    let current_line_chars = current_line.drain();
                    for char in current_line_chars {
                        self.read_buffer.lock_with(|rb| rb.push_overwrite(char));
                    }
                });
            }

            if is_printable_char(ch) {
                // Printable character
                self.current_line.lock_with(|l| l.push_char(ch));
            }

            self.update_readable_state();
        })
    }

    fn may_send_signal(&self, termios: &KernelTermios, ch: u8) -> bool {
        if !termios.is_canonical_mode() || !termios.contains_isig() {
            return false;
        }

        let signal = match ch {
            ch if ch == *termios.get_special_char(CC_C_CHAR::VINTR) => KernelSignal::new(SIGINT),
            ch if ch == *termios.get_special_char(CC_C_CHAR::VQUIT) => KernelSignal::new(SIGQUIT),
            _ => return false,
        };

        if in_interrupt_context() {
            // `kernel_signal()` may cause sleep, so only construct parameters here.
            self.work_item_para
                .lock_with(|para| para.kernel_signal = Some(signal));
        } else {
            (self.send_signal)(signal);
        }

        true
    }

    pub fn update_readable_state(&self) {
        self.read_buffer.lock_with(|buffer| {
            if !buffer.is_empty() {
                self.pollee.add_events(IoEvents::IN);
            } else {
                self.pollee.del_events(IoEvents::IN);
            }
        });
    }

    /// Sends a signal later. The signal will be handled by a work queue.
    fn send_signal_after(&self) {
        if let Some(signal) = self
            .work_item_para
            .lock_with(|para| para.kernel_signal.take())
        {
            (self.send_signal)(signal);
        };
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
                let ctrl_char = format!("^{}", get_printable_char(ch));
                echo_callback(&ctrl_char);
            }
            item => {}
        }
    }

    pub fn read(&self, buf: &mut [u8]) -> Result<usize> {
        loop {
            let res = self.try_read(buf);
            match res {
                Ok(len) => return Ok(len),
                Err(e) if e.error() != Errno::EAGAIN => return Err(e),
                Err(_) => {
                    let mut poller = Poller::new();
                    if self.poll(IoEvents::IN, Some(&mut poller)).is_empty() {
                        poller.wait()?
                    }
                }
            }
        }
    }

    /// Reads all bytes buffered to `dst`.
    ///
    /// This method returns the actual read length.
    fn try_read(&self, dst: &mut [u8]) -> Result<usize> {
        let (vmin, vtime) = self.termios.lock_with(|termios| {
            let vmin = *termios.get_special_char(CC_C_CHAR::VMIN);
            let vtime = *termios.get_special_char(CC_C_CHAR::VTIME);
            (vmin, vtime)
        });
        let read_len = {
            let len = self.read_buffer.lock_with(|rb| rb.len());
            let max_read_len = len.min(dst.len());
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
        self.update_readable_state();
        Ok(read_len)
    }

    pub fn poll(&self, mask: IoEvents, poller: Option<&mut Poller>) -> IoEvents {
        self.pollee.poll(mask, poller)
    }

    /// Reads bytes from `self` to `dst`, returning the actual bytes read.
    ///
    /// If no bytes are available, this method returns 0 immediately.
    fn poll_read(&self, dst: &mut [u8]) -> usize {
        self.read_buffer.lock_with(|buffer| {
            let len = buffer.len();
            let max_read_len = len.min(dst.len());
            if max_read_len == 0 {
                return 0;
            }
            let mut read_len = 0;
            for dst_i in dst.iter_mut().take(max_read_len) {
                if let Some(next_char) = buffer.pop() {
                    if self.termios.lock_with(|termios| {
                        if termios.is_canonical_mode() {
                            // canonical mode, read until meet new line
                            if is_line_terminator(next_char, termios) {
                                // The eof should not be read
                                if !is_eof(next_char, termios) {
                                    *dst_i = next_char;
                                    read_len += 1;
                                }
                                return true; // break
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
                        false // continue
                    }) {
                        break;
                    };
                } else {
                    break;
                }
            }

            read_len
        })
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
        let buffer_len = self.read_buffer.lock_with(|rb| rb.len());
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
        self.read_buffer.lock_with(|rb| rb.len() == 0)
    }

    pub fn termios(&self) -> KernelTermios {
        self.termios.lock_with(|termios| *termios)
    }

    pub fn set_termios(&self, termios: KernelTermios) {
        self.termios.lock_with(|t| *t = termios);
    }

    pub fn drain_input(&self) {
        self.current_line.lock_with(|l| l.drain());
        self.read_buffer.lock_with(|rb| rb.clear());
    }

    pub fn buffer_len(&self) -> usize {
        self.read_buffer.lock_with(|rb| rb.len())
    }

    pub fn window_size(&self) -> WinSize {
        self.winsize.lock_with(|ws| *ws)
    }

    pub fn set_window_size(&self, winsize: WinSize) {
        self.winsize.lock_with(|ws| *ws = winsize);
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

fn get_printable_char(ctrl_char: u8) -> char {
    debug_assert!(is_ctrl_char(ctrl_char));
    char::from_u32((ctrl_char + b'A' - 1) as u32).unwrap()
}

enum PolleeType {
    Add,
    Del,
}

struct LineDisciplineWorkPara {
    kernel_signal: Option<KernelSignal>,
}

impl LineDisciplineWorkPara {
    fn new() -> Self {
        Self {
            kernel_signal: None,
        }
    }
}
