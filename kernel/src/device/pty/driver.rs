// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use aster_console::AnyConsoleDevice;
use ostd::sync::SpinLock;

use super::file::PtySlaveFile;
use crate::{
    device::tty::{Tty, TtyDriver},
    events::IoEvents,
    fs::inode_handle::FileIo,
    prelude::*,
    process::signal::Pollee,
    util::ring_buffer::RingBuffer,
};

const BUFFER_CAPACITY: usize = 8192;

/// A pseudoterminal driver.
///
/// This is contained in the pty slave, but it maintains the output buffer and the pollee of the
/// master. The pollee of the slave is part of the [`Tty`] structure (see the definition of
/// [`PtySlave`]).
pub struct PtyDriver {
    output: SpinLock<RingBuffer<u8>>,
    pollee: Pollee,
    is_master_closed: AtomicBool,
    opened_slaves: AtomicUsize,
}

/// A pseudoterminal slave.
pub type PtySlave = Tty<PtyDriver>;

impl PtyDriver {
    pub(super) fn new() -> Self {
        Self {
            output: SpinLock::new(RingBuffer::new(BUFFER_CAPACITY)),
            pollee: Pollee::new(),
            is_master_closed: AtomicBool::new(false),
            opened_slaves: AtomicUsize::new(0),
        }
    }

    pub(super) fn try_read(&self, buf: &mut [u8]) -> Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }

        let mut output = self.output.lock();
        if output.is_empty() {
            if self.opened_slaves.load(Ordering::Relaxed) == 0 {
                return_errno_with_message!(
                    Errno::EIO,
                    "the pty master does not have opened slaves"
                );
            }
            return_errno_with_message!(Errno::EAGAIN, "the buffer is empty");
        }

        let read_len = output.len().min(buf.len());
        output.pop_slice(&mut buf[..read_len]).unwrap();

        Ok(read_len)
    }

    pub(super) fn pollee(&self) -> &Pollee {
        &self.pollee
    }

    pub(super) fn buffer_len(&self) -> usize {
        self.output.lock().len()
    }

    pub(super) fn set_master_closed(&self) {
        self.is_master_closed.store(true, Ordering::Relaxed);
    }

    pub(super) fn opened_slaves(&self) -> &AtomicUsize {
        &self.opened_slaves
    }
}

impl TtyDriver for PtyDriver {
    // Reference: <https://elixir.bootlin.com/linux/v6.17/source/include/uapi/linux/major.h#L147>.
    const DEVICE_MAJOR_ID: u32 = 136;

    fn open(tty: Arc<Tty<Self>>) -> Arc<dyn FileIo> {
        Arc::new(PtySlaveFile::new(tty))
    }

    fn push_output(&self, chs: &[u8]) -> Result<usize> {
        if self.is_closed() {
            return_errno_with_message!(Errno::EIO, "the pty master has been closed");
        }

        let mut output = self.output.lock();

        let mut len = 0;
        for ch in chs {
            // TODO: This is termios-specific behavior and should be part of the TTY implementation
            // instead of the TTY driver implementation. See the ONLCR flag for more details.
            if *ch == b'\n' && output.capacity() - output.len() >= 2 {
                output.push(b'\r').unwrap();
                output.push(b'\n').unwrap();
            } else if *ch != b'\n' && !output.is_full() {
                output.push(*ch).unwrap();
            } else if len == 0 {
                return_errno_with_message!(Errno::EAGAIN, "the output buffer is full");
            } else {
                break;
            }
            len += 1;
        }

        self.pollee.notify(IoEvents::IN);
        Ok(len)
    }

    fn drain_output(&self) {
        self.output.lock().clear();
        self.pollee.invalidate();
    }

    fn echo_callback(&self) -> impl FnMut(&[u8]) + '_ {
        let mut output = self.output.lock();
        let mut has_notified = false;

        move |chs| {
            for ch in chs {
                let _ = output.push(*ch);
            }

            if !has_notified {
                self.pollee.notify(IoEvents::IN);
                has_notified = true;
            }
        }
    }

    fn can_push(&self) -> bool {
        let output = self.output.lock();
        output.capacity() - output.len() >= 2
    }

    fn is_closed(&self) -> bool {
        self.is_master_closed.load(Ordering::Relaxed)
    }

    fn notify_input(&self) {
        self.pollee.notify(IoEvents::OUT);
    }

    fn console(&self) -> Option<&dyn AnyConsoleDevice> {
        None
    }
}
