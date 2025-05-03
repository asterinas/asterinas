// SPDX-License-Identifier: MPL-2.0

use ostd::sync::SpinLock;

use crate::{
    device::tty::{Tty, TtyDriver},
    events::IoEvents,
    prelude::{return_errno_with_message, Errno, Result},
    process::signal::Pollee,
    util::ring_buffer::RingBuffer,
};

const BUFFER_CAPACITY: usize = 4096;

/// A pseudoterminal driver.
///
/// This is contained in the PTY slave, but it maintains the output buffer and the pollee of the
/// master. The pollee of the slave is part of the [`Tty`] structure (see the definition of
/// [`PtySlave`]).
pub struct PtyDriver {
    output: SpinLock<RingBuffer<u8>>,
    pollee: Pollee,
}

/// A pseudoterminal slave.
pub type PtySlave = Tty<PtyDriver>;

impl PtyDriver {
    pub(super) fn new() -> Self {
        Self {
            output: SpinLock::new(RingBuffer::new(BUFFER_CAPACITY)),
            pollee: Pollee::new(),
        }
    }

    pub(super) fn try_read(&self, buf: &mut [u8]) -> Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }

        let mut output = self.output.lock();
        if output.is_empty() {
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
}

impl TtyDriver for PtyDriver {
    fn push_output(&self, chs: &[u8]) {
        let mut output = self.output.lock();

        for ch in chs {
            // TODO: This is termios-specific behavior and should be part of the TTY implementation
            // instead of the TTY driver implementation. See the ONLCR flag for more details.
            if *ch == b'\n' {
                output.push_overwrite(b'\r');
                output.push_overwrite(b'\n');
                continue;
            }
            output.push_overwrite(*ch);
        }
        self.pollee.notify(IoEvents::IN);
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
                output.push_overwrite(*ch);
            }

            if !has_notified {
                self.pollee.notify(IoEvents::IN);
                has_notified = true;
            }
        }
    }
}
