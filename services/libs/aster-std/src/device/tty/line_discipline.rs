use crate::events::IoEvents;
use crate::prelude::*;
use crate::process::signal::constants::{SIGINT, SIGQUIT};
use crate::process::signal::signals::kernel::KernelSignal;
use crate::process::signal::{Pollee, Poller};
use crate::thread::work_queue::work_item::WorkItem;
use crate::thread::work_queue::{submit_work_item, WorkPriority};
use alloc::format;
use aster_frame::trap::disable_local;
use ringbuf::{ring_buffer::RbBase, Rb, StaticRb};

use super::termio::{KernelTermios, WinSize, CC_C_CHAR};

// This implementation refers the implementation of linux
// https://elixir.bootlin.com/linux/latest/source/include/linux/tty_ldisc.h

const BUFFER_CAPACITY: usize = 4096;

pub type LdiscSignalSender = Arc<dyn Fn(KernelSignal) + Send + Sync + 'static>;

pub struct LineDiscipline {
    /// current line
    current_line: SpinLock<CurrentLine>,
    /// The read buffer
    read_buffer: SpinLock<StaticRb<u8, BUFFER_CAPACITY>>,
    /// termios
    termios: SpinLock<KernelTermios>,
    /// Windows size,
    winsize: SpinLock<WinSize>,
    /// Pollee
    pollee: Pollee,
    /// Used to send signal for foreground processes, when some char comes.
    send_signal: LdiscSignalSender,
    /// work item
    work_item: Arc<WorkItem>,
    /// Parameters used by a work item.
    work_item_para: Arc<SpinLock<LineDisciplineWorkPara>>,
}

#[derive(Default)]
pub struct CurrentLine {
    buffer: StaticRb<u8, BUFFER_CAPACITY>,
}

impl CurrentLine {
    pub fn new() -> Self {
        Self {
            buffer: StaticRb::default(),
        }
    }

    /// read all bytes inside current line and clear current line
    pub fn drain(&mut self) -> Vec<u8> {
        self.buffer.pop_iter().collect()
    }

    pub fn push_char(&mut self, char: u8) {
        // What should we do if line is full?
        debug_assert!(!self.is_full());
        self.buffer.push_overwrite(char);
    }

    pub fn backspace(&mut self) {
        self.buffer.pop();
    }

    pub fn is_full(&self) -> bool {
        self.buffer.is_full()
    }

    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }
}

impl LineDiscipline {
    /// Create a new line discipline
    pub fn new(send_signal: LdiscSignalSender) -> Arc<Self> {
        Arc::new_cyclic(move |line_ref: &Weak<LineDiscipline>| {
            let line_discipline = line_ref.clone();
            let work_item = Arc::new(WorkItem::new(Box::new(move || {
                if let Some(line_discipline) = line_discipline.upgrade() {
                    line_discipline.update_readable_state_after();
                }
            })));
            Self {
                current_line: SpinLock::new(CurrentLine::new()),
                read_buffer: SpinLock::new(StaticRb::default()),
                termios: SpinLock::new(KernelTermios::default()),
                winsize: SpinLock::new(WinSize::default()),
                pollee: Pollee::new(IoEvents::empty()),
                send_signal,
                work_item,
                work_item_para: Arc::new(SpinLock::new(LineDisciplineWorkPara::new())),
            }
        })
    }

    /// Push char to line discipline.
    pub fn push_char<F2: FnMut(&str)>(&self, ch: u8, echo_callback: F2) {
        let termios = self.termios.lock_irq_disabled();

        let ch = if termios.contains_icrnl() && ch == b'\r' {
            b'\n'
        } else {
            ch
        };

        if self.may_send_signal(&termios, ch) {
            // The char is already dealt with, so just return
            return;
        }

        // Typically, a tty in raw mode does not echo. But the tty can also be in a cbreak mode,
        // with ICANON closed and ECHO opened.
        if termios.contain_echo() {
            self.output_char(ch, &termios, echo_callback);
        }

        // Raw mode
        if !termios.is_canonical_mode() {
            self.read_buffer.lock_irq_disabled().push_overwrite(ch);
            self.update_readable_state_deferred();
            return;
        }

        // Canonical mode

        if ch == *termios.get_special_char(CC_C_CHAR::VKILL) {
            // Erase current line
            self.current_line.lock_irq_disabled().drain();
        }

        if ch == *termios.get_special_char(CC_C_CHAR::VERASE) {
            // Type backspace
            let mut current_line = self.current_line.lock_irq_disabled();
            if !current_line.is_empty() {
                current_line.backspace();
            }
        }

        if is_line_terminator(ch, &termios) {
            // If a new line is met, all bytes in current_line will be moved to read_buffer
            let mut current_line = self.current_line.lock_irq_disabled();
            current_line.push_char(ch);
            let current_line_chars = current_line.drain();
            for char in current_line_chars {
                self.read_buffer.lock_irq_disabled().push_overwrite(char);
            }
        }

        if is_printable_char(ch) {
            // Printable character
            self.current_line.lock_irq_disabled().push_char(ch);
        }

        self.update_readable_state_deferred();
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
        // `kernel_signal()` may cause sleep, so only construct parameters here.
        self.work_item_para.lock_irq_disabled().kernel_signal = Some(signal);

        true
    }

    pub fn update_readable_state(&self) {
        let buffer = self.read_buffer.lock_irq_disabled();
        if !buffer.is_empty() {
            self.pollee.add_events(IoEvents::IN);
        } else {
            self.pollee.del_events(IoEvents::IN);
        }
    }

    fn update_readable_state_deferred(&self) {
        let buffer = self.read_buffer.lock_irq_disabled();
        // add/del events may sleep, so only construct parameters here.
        if !buffer.is_empty() {
            self.work_item_para.lock_irq_disabled().pollee_type = Some(PolleeType::Add);
        } else {
            self.work_item_para.lock_irq_disabled().pollee_type = Some(PolleeType::Del);
        }
        submit_work_item(self.work_item.clone(), WorkPriority::High);
    }

    /// include all operations that may cause sleep, and processes by a work queue.
    fn update_readable_state_after(&self) {
        if let Some(signal) = self.work_item_para.lock_irq_disabled().kernel_signal.take() {
            (self.send_signal)(signal);
        };
        if let Some(pollee_type) = self.work_item_para.lock_irq_disabled().pollee_type.take() {
            match pollee_type {
                PolleeType::Add => {
                    self.pollee.add_events(IoEvents::IN);
                }
                PolleeType::Del => {
                    self.pollee.del_events(IoEvents::IN);
                }
            }
        }
    }

    // TODO: respect output flags
    fn output_char<F: FnMut(&str)>(&self, ch: u8, termios: &KernelTermios, mut echo_callback: F) {
        match ch {
            b'\n' => echo_callback("\n"),
            b'\r' => echo_callback("\r\n"),
            ch if ch == *termios.get_special_char(CC_C_CHAR::VERASE) => {
                // write a space to overwrite current character
                let backspace: &str = core::str::from_utf8(&[b'\x08', b' ', b'\x08']).unwrap();
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
                    let poller = Some(Poller::new());
                    if self.poll(IoEvents::IN, poller.as_ref()).is_empty() {
                        poller.as_ref().unwrap().wait()?
                    }
                }
            }
        }
    }

    /// read all bytes buffered to dst, return the actual read length.
    fn try_read(&self, dst: &mut [u8]) -> Result<usize> {
        let (vmin, vtime) = {
            let termios = self.termios.lock_irq_disabled();
            let vmin = *termios.get_special_char(CC_C_CHAR::VMIN);
            let vtime = *termios.get_special_char(CC_C_CHAR::VTIME);
            (vmin, vtime)
        };
        let read_len = {
            let len = self.read_buffer.lock_irq_disabled().len();
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

    pub fn poll(&self, mask: IoEvents, poller: Option<&Poller>) -> IoEvents {
        self.pollee.poll(mask, poller)
    }

    /// returns immediately with the lesser of the number of bytes available or the number of bytes requested.
    /// If no bytes are available, completes immediately, returning 0.
    fn poll_read(&self, dst: &mut [u8]) -> usize {
        let mut buffer = self.read_buffer.lock_irq_disabled();
        let len = buffer.len();
        let max_read_len = len.min(dst.len());
        if max_read_len == 0 {
            return 0;
        }
        let mut read_len = 0;
        for dst_i in dst.iter_mut().take(max_read_len) {
            if let Some(next_char) = buffer.pop() {
                let termios = self.termios.lock_irq_disabled();
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
                    // FIXME: avoid addtional bound check
                    *dst_i = next_char;
                    read_len += 1;
                }
            } else {
                break;
            }
        }

        read_len
    }

    // The read() blocks until the number of bytes requested or
    // at least vmin bytes are available, and returns the real read value.
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

    /// write bytes to buffer, if flush to console, then write the content to console
    pub fn write(&self, src: &[u8], flush_to_console: bool) -> Result<usize> {
        todo!()
    }

    /// whether there is buffered data
    pub fn is_empty(&self) -> bool {
        self.read_buffer.lock_irq_disabled().len() == 0
    }

    pub fn termios(&self) -> KernelTermios {
        *self.termios.lock_irq_disabled()
    }

    pub fn set_termios(&self, termios: KernelTermios) {
        *self.termios.lock_irq_disabled() = termios;
    }

    pub fn drain_input(&self) {
        self.current_line.lock().drain();
        let _: Vec<_> = self.read_buffer.lock().pop_iter().collect();
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

fn get_printable_char(ctrl_char: u8) -> u8 {
    debug_assert!(is_ctrl_char(ctrl_char));
    ctrl_char + b'A' - 1
}

enum PolleeType {
    Add,
    Del,
}

struct LineDisciplineWorkPara {
    #[allow(clippy::type_complexity)]
    kernel_signal: Option<KernelSignal>,
    pollee_type: Option<PolleeType>,
}

impl LineDisciplineWorkPara {
    fn new() -> Self {
        Self {
            kernel_signal: None,
            pollee_type: None,
        }
    }
}
