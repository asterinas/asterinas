use crate::fs::utils::{IoEvents, Pollee, Poller};
use crate::process::signal::constants::{SIGINT, SIGQUIT};
use crate::{
    prelude::*,
    process::{process_table, signal::signals::kernel::KernelSignal, Pgid},
};
use ringbuf::{ring_buffer::RbBase, Rb, StaticRb};

use super::termio::{KernelTermios, CC_C_CHAR};

// This implementation refers the implementation of linux
// https://elixir.bootlin.com/linux/latest/source/include/linux/tty_ldisc.h

const BUFFER_CAPACITY: usize = 4096;

pub struct LineDiscipline {
    /// current line
    current_line: SpinLock<CurrentLine>,
    /// The read buffer
    read_buffer: SpinLock<StaticRb<u8, BUFFER_CAPACITY>>,
    /// The foreground process group
    foreground: SpinLock<Option<Pgid>>,
    /// termios
    termios: SpinLock<KernelTermios>,
    /// Pollee
    pollee: Pollee,
}

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
    /// create a new line discipline
    pub fn new() -> Self {
        Self {
            current_line: SpinLock::new(CurrentLine::new()),
            read_buffer: SpinLock::new(StaticRb::default()),
            foreground: SpinLock::new(None),
            termios: SpinLock::new(KernelTermios::default()),
            pollee: Pollee::new(IoEvents::empty()),
        }
    }

    /// push char to line discipline. This function should be called in input interrupt handler.
    pub fn push_char(&self, mut item: u8) {
        let termios = self.termios.lock_irq_disabled();
        if termios.contains_icrnl() {
            if item == b'\r' {
                item = b'\n'
            }
        }
        if termios.is_canonical_mode() {
            if item == *termios.get_special_char(CC_C_CHAR::VINTR) {
                // type Ctrl + C, signal SIGINT
                if termios.contains_isig() {
                    if let Some(fg) = *self.foreground.lock_irq_disabled() {
                        let kernel_signal = KernelSignal::new(SIGINT);
                        let fg_group = process_table::pgid_to_process_group(fg).unwrap();
                        fg_group.kernel_signal(kernel_signal);
                    }
                }
            } else if item == *termios.get_special_char(CC_C_CHAR::VQUIT) {
                // type Ctrl + \, signal SIGQUIT
                if termios.contains_isig() {
                    if let Some(fg) = *self.foreground.lock_irq_disabled() {
                        let kernel_signal = KernelSignal::new(SIGQUIT);
                        let fg_group = process_table::pgid_to_process_group(fg).unwrap();
                        fg_group.kernel_signal(kernel_signal);
                    }
                }
            } else if item == *termios.get_special_char(CC_C_CHAR::VKILL) {
                // erase current line
                self.current_line.lock_irq_disabled().drain();
            } else if item == *termios.get_special_char(CC_C_CHAR::VERASE) {
                // type backspace
                let mut current_line = self.current_line.lock_irq_disabled();
                if !current_line.is_empty() {
                    current_line.backspace();
                }
            } else if meet_new_line(item, &termios) {
                // a new line was met. We currently add the item to buffer.
                // when we read content, the item should be skipped if it's EOF.
                let mut current_line = self.current_line.lock_irq_disabled();
                current_line.push_char(item);
                let current_line_chars = current_line.drain();
                for char in current_line_chars {
                    self.read_buffer.lock_irq_disabled().push_overwrite(char);
                }
            } else if item >= 0x20 && item < 0x7f {
                // printable character
                self.current_line.lock_irq_disabled().push_char(item);
            }
        } else {
            // raw mode
            self.read_buffer.lock_irq_disabled().push_overwrite(item);
            // debug!("push char: {}", char::from(item))
        }

        if termios.contain_echo() {
            self.output_char(item, &termios);
        }

        if self.is_readable() {
            self.pollee.add_events(IoEvents::IN);
        }
    }

    /// whether self is readable
    fn is_readable(&self) -> bool {
        !self.read_buffer.lock_irq_disabled().is_empty()
    }

    // TODO: respect output flags
    fn output_char(&self, item: u8, termios: &KernelTermios) {
        match item {
            b'\n' => print!("\n"),
            b'\r' => print!("\r\n"),
            item if item == *termios.get_special_char(CC_C_CHAR::VERASE) => {
                // write a space to overwrite current character
                let backspace: &str = core::str::from_utf8(&[b'\x08', b' ', b'\x08']).unwrap();
                print!("{}", backspace);
            }
            item if 0x20 <= item && item < 0x7f => print!("{}", char::from(item)),
            item if 0 < item && item < 0x20 && termios.contains_echo_ctl() => {
                // The unprintable chars between 1-31 are mapped to ctrl characters between 65-95.
                // e.g., 0x3 is mapped to 0x43, which is C. So, we will print ^C when 0x3 is met.
                if 0 < item && item < 0x20 {
                    let ctrl_char = char::from(item + 0x40);
                    print!("^{ctrl_char}");
                }
            }
            item => {}
        }
    }

    /// read all bytes buffered to dst, return the actual read length.
    pub fn read(&self, dst: &mut [u8]) -> Result<usize> {
        let mut poller = None;
        loop {
            let res = self.try_read(dst);
            match res {
                Ok(read_len) => {
                    return Ok(read_len);
                }
                Err(e) => {
                    if e.error() != Errno::EAGAIN {
                        return Err(e);
                    }
                }
            }

            // Wait for read event
            let need_poller = if poller.is_none() {
                poller = Some(Poller::new());
                poller.as_ref()
            } else {
                None
            };
            let revents = self.pollee.poll(IoEvents::IN, need_poller);
            if revents.is_empty() {
                poller.as_ref().unwrap().wait();
            }
        }
    }

    pub fn try_read(&self, dst: &mut [u8]) -> Result<usize> {
        if !self.current_belongs_to_foreground() {
            return_errno!(Errno::EAGAIN);
        }

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
        if !self.is_readable() {
            self.pollee.del_events(IoEvents::IN);
        }
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
        for i in 0..max_read_len {
            if let Some(next_char) = buffer.pop() {
                let termios = self.termios.lock_irq_disabled();
                if termios.is_canonical_mode() {
                    // canonical mode, read until meet new line
                    if meet_new_line(next_char, &termios) {
                        if !should_not_be_read(next_char, &termios) {
                            dst[i] = next_char;
                            read_len += 1;
                        }
                        break;
                    } else {
                        dst[i] = next_char;
                        read_len += 1;
                    }
                } else {
                    // raw mode
                    // FIXME: avoid addtional bound check
                    dst[i] = next_char;
                    read_len += 1;
                }
            } else {
                break;
            }
        }

        read_len
    }

    // The read() blocks until the lesser of the number of bytes requested or
    // MIN bytes are available, and returns the lesser of the two values.
    pub fn block_read(&self, dst: &mut [u8], vmin: u8) -> Result<usize> {
        let min_read_len = (vmin as usize).min(dst.len());
        let buffer_len = self.read_buffer.lock_irq_disabled().len();
        if buffer_len < min_read_len {
            return_errno!(Errno::EAGAIN);
        }
        Ok(self.poll_read(&mut dst[..min_read_len]))
    }

    /// write bytes to buffer, if flush to console, then write the content to console
    pub fn write(&self, src: &[u8], flush_to_console: bool) -> Result<usize> {
        todo!()
    }

    /// whether the current process belongs to foreground process group
    fn current_belongs_to_foreground(&self) -> bool {
        let current = current!();
        if let Some(fg_pgid) = *self.foreground.lock_irq_disabled() {
            if let Some(process_group) = process_table::pgid_to_process_group(fg_pgid) {
                if process_group.contains_process(current.pid()) {
                    return true;
                }
            }
        }

        false
    }

    /// set foreground process group
    pub fn set_fg(&self, fg_pgid: Pgid) {
        *self.foreground.lock_irq_disabled() = Some(fg_pgid);
        // Some background processes may be waiting on the wait queue, when set_fg, the background processes may be able to read.
        if self.is_readable() {
            self.pollee.add_events(IoEvents::IN);
        }
    }

    /// get foreground process group id
    pub fn fg_pgid(&self) -> Option<Pgid> {
        *self.foreground.lock_irq_disabled()
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
}

fn meet_new_line(item: u8, termios: &KernelTermios) -> bool {
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

/// The special char should not be read by reading process
fn should_not_be_read(item: u8, termios: &KernelTermios) -> bool {
    if item == *termios.get_special_char(CC_C_CHAR::VEOF) {
        true
    } else {
        false
    }
}
