use crate::{prelude::*, process::Pgid};
use ringbuffer::{ConstGenericRingBuffer, RingBuffer, RingBufferRead, RingBufferWrite};

use super::termio::KernelTermios;

// This implementation refers the implementation of linux
// https://elixir.bootlin.com/linux/latest/source/include/linux/tty_ldisc.h

const BUFFER_CAPACITY: usize = 4096;

#[derive(Debug)]
pub struct LineDiscipline {
    /// The write buffer
    buffer: ConstGenericRingBuffer<u8, BUFFER_CAPACITY>,
    /// The foreground process group
    foreground: Option<Pgid>,
    /// termios
    termios: KernelTermios,
}

impl LineDiscipline {
    /// create a new line discipline
    pub fn new() -> Self {
        Self {
            buffer: ConstGenericRingBuffer::new(),
            foreground: None,
            termios: KernelTermios::default(),
        }
    }

    /// push char to buffer
    pub fn push_char(&mut self, mut item: u8) {
        if self.termios.is_cooked_mode() {
            todo!("We only support raw mode now. Cooked mode will be supported further.");
        }
        if self.termios.contains_icrnl() {
            if item == b'\r' {
                item = b'\n'
            }
        }
        self.buffer.push(item);
    }

    /// read all bytes buffered to dst, return the actual read length.
    pub fn read(&mut self, dst: &mut [u8]) -> Result<usize> {
        let len = self.buffer.len();
        let read_len = len.min(dst.len());
        for i in 0..read_len {
            if let Some(content) = self.buffer.dequeue() {
                dst[i] = content;
            } else {
                break;
            }
        }
        Ok(read_len)
    }

    /// write bytes to buffer, if flush to console, then write the content to console
    pub fn write(&self, src: &[u8], flush_to_console: bool) -> Result<usize> {
        todo!()
    }

    /// set foreground process group
    pub fn set_fg(&mut self, fg_pgid: Pgid) {
        self.foreground = Some(fg_pgid);
    }

    /// get foreground process group id
    pub fn get_fg(&self) -> Option<&Pgid> {
        self.foreground.as_ref()
    }

    /// whether there is buffered data
    pub fn is_empty(&self) -> bool {
        self.buffer.len() == 0
    }

    pub fn get_termios(&self) -> &KernelTermios {
        &self.termios
    }

    pub fn set_termios(&mut self, termios: KernelTermios) {
        self.termios = termios;
    }
}
