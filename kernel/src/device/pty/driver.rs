// SPDX-License-Identifier: MPL-2.0

use aster_console::AnyConsoleDevice;
use ostd::sync::SpinLock;

use super::file::PtySlaveFile;
use crate::{
    device::tty::{Tty, TtyDriver, TtyFlags},
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
    opened_slaves: SpinLock<usize>,
    tty_flags: TtyFlags,
}

/// A pseudoterminal slave.
pub type PtySlave = Tty<PtyDriver>;

impl PtyDriver {
    pub(super) fn new() -> Self {
        let tty_flags = TtyFlags::new();
        tty_flags.set_pty_locked();
        Self {
            output: SpinLock::new(RingBuffer::new(BUFFER_CAPACITY)),
            pollee: Pollee::new(),
            opened_slaves: SpinLock::new(0),
            tty_flags,
        }
    }

    pub(super) fn try_read(&self, buf: &mut [u8]) -> Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }

        let mut output = self.output.lock();
        if output.is_empty() {
            if self.tty_flags.is_other_closed() {
                return_errno_with_message!(Errno::EIO, "the pty slave has been closed");
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

    pub(super) fn opened_slaves(&self) -> &SpinLock<usize> {
        &self.opened_slaves
    }

    pub(super) fn tty_flags(&self) -> &TtyFlags {
        &self.tty_flags
    }
}

impl TtyDriver for PtyDriver {
    // Reference: <https://elixir.bootlin.com/linux/v6.17/source/include/uapi/linux/major.h#L147>.
    const DEVICE_MAJOR_ID: u32 = 136;

    fn devtmpfs_path(&self, _index: u32) -> Option<String> {
        None
    }

    fn open(tty: Arc<Tty<Self>>) -> Result<Box<dyn FileIo>> {
        Ok(Box::new(PtySlaveFile::new(tty)?))
    }

    fn push_output(&self, chs: &[u8]) -> Result<usize> {
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

    fn notify_input(&self) {
        self.pollee.notify(IoEvents::OUT);
    }

    fn console(&self) -> Option<&dyn AnyConsoleDevice> {
        None
    }
}
