use crate::fs::utils::IoEvents;
use crate::process::signal::constants::{SIGINT, SIGQUIT};
use crate::{
    prelude::*,
    process::{process_table, signal::signals::kernel::KernelSignal, Pgid},
};
use jinux_frame::sync::WaitQueue;
use ringbuffer::{ConstGenericRingBuffer, RingBuffer, RingBufferRead, RingBufferWrite};

use super::termio::{KernelTermios, CC_C_CHAR};

// This implementation refers the implementation of linux
// https://elixir.bootlin.com/linux/latest/source/include/linux/tty_ldisc.h

const BUFFER_CAPACITY: usize = 4096;

pub struct LineDiscipline {
    /// current line
    current_line: RwLock<CurrentLine>,
    /// The read buffer
    read_buffer: Mutex<ConstGenericRingBuffer<u8, BUFFER_CAPACITY>>,
    /// The foreground process group
    foreground: RwLock<Option<Pgid>>,
    /// termios
    termios: RwLock<KernelTermios>,
    /// wait until self is readable
    read_wait_queue: WaitQueue,
}

#[derive(Debug)]
pub struct CurrentLine {
    buffer: ConstGenericRingBuffer<u8, BUFFER_CAPACITY>,
}

impl CurrentLine {
    pub fn new() -> Self {
        Self {
            buffer: ConstGenericRingBuffer::new(),
        }
    }

    /// read all bytes inside current line and clear current line
    pub fn drain(&mut self) -> Vec<u8> {
        self.buffer.drain().collect()
    }

    pub fn push_char(&mut self, char: u8) {
        // What should we do if line is full?
        debug_assert!(!self.is_full());
        self.buffer.push(char);
    }

    pub fn backspace(&mut self) {
        self.buffer.dequeue();
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
            current_line: RwLock::new(CurrentLine::new()),
            read_buffer: Mutex::new(ConstGenericRingBuffer::new()),
            foreground: RwLock::new(None),
            termios: RwLock::new(KernelTermios::default()),
            read_wait_queue: WaitQueue::new(),
        }
    }

    /// push char to line discipline. This function should be called in input interrupt handler.
    pub fn push_char(&self, mut item: u8) {
        let termios = self.termios.read();
        if termios.contains_icrnl() {
            if item == b'\r' {
                item = b'\n'
            }
        }
        if termios.is_canonical_mode() {
            if item == *termios.get_special_char(CC_C_CHAR::VINTR) {
                // type Ctrl + C, signal SIGINT
                if termios.contains_isig() {
                    if let Some(fg) = *self.foreground.read() {
                        let kernel_signal = KernelSignal::new(SIGINT);
                        let fg_group = process_table::pgid_to_process_group(fg).unwrap();
                        fg_group.kernel_signal(kernel_signal);
                    }
                }
            } else if item == *termios.get_special_char(CC_C_CHAR::VQUIT) {
                // type Ctrl + \, signal SIGQUIT
                if termios.contains_isig() {
                    if let Some(fg) = *self.foreground.read() {
                        let kernel_signal = KernelSignal::new(SIGQUIT);
                        let fg_group = process_table::pgid_to_process_group(fg).unwrap();
                        fg_group.kernel_signal(kernel_signal);
                    }
                }
            } else if item == *termios.get_special_char(CC_C_CHAR::VKILL) {
                // erase current line
                self.current_line.write().drain();
            } else if item == *termios.get_special_char(CC_C_CHAR::VERASE) {
                // type backspace
                let mut current_line = self.current_line.write();
                if !current_line.is_empty() {
                    current_line.backspace();
                }
            } else if meet_new_line(item, &self.get_termios()) {
                // a new line was met. We currently add the item to buffer.
                // when we read content, the item should be skipped if it's EOF.
                let mut current_line = self.current_line.write();
                current_line.push_char(item);
                let current_line_chars = current_line.drain();
                for char in current_line_chars {
                    self.read_buffer.lock().push(char);
                }
            } else if item >= 0x20 && item < 0x7f {
                // printable character
                self.current_line.write().push_char(item);
            }
        } else {
            // raw mode
            self.read_buffer.lock().push(item);
            // debug!("push char: {}", char::from(item))
        }

        if termios.contain_echo() {
            self.output_char(item);
        }

        if self.is_readable() {
            self.read_wait_queue.wake_all();
        }
    }

    /// whether self is readable
    fn is_readable(&self) -> bool {
        self.read_buffer.lock().len() > 0
    }

    // TODO: respect output flags
    fn output_char(&self, item: u8) {
        if 0x20 <= item && item < 0x7f {
            let ch = char::from(item);
            print!("{}", ch);
        }
        let termios = self.termios.read();
        if item == *termios.get_special_char(CC_C_CHAR::VERASE) {
            // write a space to overwrite current character
            let bytes: [u8; 3] = [b'\x08', b' ', b'\x08'];
            let backspace = core::str::from_utf8(&bytes).unwrap();
            print!("{}", backspace);
        }
        if termios.contains_echo_ctl() {
            // The unprintable chars between 1-31 are mapped to ctrl characters between 65-95.
            // e.g., 0x3 is mapped to 0x43, which is C. So, we will print ^C when 0x3 is met.
            if 0 < item && item < 0x20 {
                let ctrl_char_ascii = item + 0x40;
                let ctrl_char = char::from(ctrl_char_ascii);
                print!("^{ctrl_char}");
            }
        }
    }

    /// read all bytes buffered to dst, return the actual read length.
    pub fn read(&self, dst: &mut [u8]) -> Result<usize> {
        let termios = self.termios.read();
        let vmin = *termios.get_special_char(CC_C_CHAR::VMIN);
        let vtime = *termios.get_special_char(CC_C_CHAR::VTIME);
        drop(termios);
        let read_len: usize = self.read_wait_queue.wait_until(|| {
            // if current process does not belong to foreground process group,
            // block until current process become foreground.
            if !self.current_belongs_to_foreground() {
                warn!("current process does not belong to foreground process group");
                return None;
            }
            let len = self.read_buffer.lock().len();
            let max_read_len = len.min(dst.len());
            if vmin == 0 && vtime == 0 {
                // poll read
                return self.poll_read(dst);
            }
            if vmin > 0 && vtime == 0 {
                // block read
                return self.block_read(dst, vmin);
            }
            if vmin == 0 && vtime > 0 {
                todo!()
            }
            if vmin > 0 && vtime > 0 {
                todo!()
            }
            unreachable!()
        });
        Ok(read_len)
    }

    pub fn poll(&self) -> IoEvents {
        if self.is_empty() {
            IoEvents::empty()
        } else {
            IoEvents::POLLIN
        }
    }

    /// returns immediately with the lesser of the number of bytes available or the number of bytes requested.
    /// If no bytes are available, completes immediately, returning 0.
    fn poll_read(&self, dst: &mut [u8]) -> Option<usize> {
        let mut buffer = self.read_buffer.lock();
        let len = buffer.len();
        let max_read_len = len.min(dst.len());
        if max_read_len == 0 {
            return Some(0);
        }
        let mut read_len = 0;
        for i in 0..max_read_len {
            if let Some(next_char) = buffer.dequeue() {
                if self.termios.read().is_canonical_mode() {
                    // canonical mode, read until meet new line
                    if meet_new_line(next_char, &self.termios.read()) {
                        if !should_not_be_read(next_char, &self.termios.read()) {
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

        Some(read_len)
    }

    // The read() blocks until the lesser of the number of bytes requested or
    // MIN bytes are available, and returns the lesser of the two values.
    pub fn block_read(&self, dst: &mut [u8], vmin: u8) -> Option<usize> {
        let min_read_len = (vmin as usize).min(dst.len());
        let buffer_len = self.read_buffer.lock().len();
        if buffer_len < min_read_len {
            return None;
        }
        return self.poll_read(&mut dst[..min_read_len]);
    }

    /// write bytes to buffer, if flush to console, then write the content to console
    pub fn write(&self, src: &[u8], flush_to_console: bool) -> Result<usize> {
        todo!()
    }

    /// whether the current process belongs to foreground process group
    fn current_belongs_to_foreground(&self) -> bool {
        let current = current!();
        if let Some(fg_pgid) = *self.foreground.read() {
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
        *self.foreground.write() = Some(fg_pgid);
        // Some background processes may be waiting on the wait queue, when set_fg, the background processes may be able to read.
        if self.is_readable() {
            self.read_wait_queue.wake_all();
        }
    }

    /// get foreground process group id
    pub fn get_fg(&self) -> Option<Pgid> {
        *self.foreground.read()
    }

    /// whether there is buffered data
    pub fn is_empty(&self) -> bool {
        self.read_buffer.lock().len() == 0
    }

    pub fn get_termios(&self) -> KernelTermios {
        *self.termios.read()
    }

    pub fn set_termios(&self, termios: KernelTermios) {
        *self.termios.write() = termios;
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
