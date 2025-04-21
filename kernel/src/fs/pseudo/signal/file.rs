// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicBool, Ordering};

use crate::{
    events::{IoEvents, Observer},
    fs::{
        file_handle::FileLike,
        path::Dentry,
        pseudo::alloc_anon_dentry,
        utils::{Metadata, StatusFlags},
    },
    prelude::*,
    process::{
        posix_thread::AsPosixThread,
        signal::{
            sig_mask::{AtomicSigMask, SigMask},
            signals::Signal,
            PollHandle, Pollable, Pollee, SigEvents, SigEventsFilter,
        },
    },
};

/// Signal file implementation
///
/// Represents a file that can be used to receive signals
/// as readable events.
pub struct SignalFile {
    /// Atomic signal mask for filtering signals
    signals_mask: AtomicSigMask,
    /// I/O event notifier
    pollee: Pollee,
    /// Non-blocking mode flag
    non_blocking: AtomicBool,
    /// Weak reference to self as an observer
    weak_self: Weak<dyn Observer<SigEvents>>,
    dentry: Dentry,
}

impl SignalFile {
    /// Create a new signalfd instance
    pub fn new(mask: AtomicSigMask, non_blocking: bool) -> Arc<Self> {
        let dentry = alloc_anon_dentry("[signalfd]").unwrap();
        Arc::new_cyclic(|weak_ref| {
            let weak_self = weak_ref.clone() as Weak<dyn Observer<SigEvents>>;
            Self {
                signals_mask: mask,
                pollee: Pollee::new(),
                non_blocking: AtomicBool::new(non_blocking),
                weak_self,
                dentry,
            }
        })
    }

    pub fn mask(&self) -> &AtomicSigMask {
        &self.signals_mask
    }

    pub fn observer_ref(&self) -> Weak<dyn Observer<SigEvents>> {
        self.weak_self.clone()
    }

    pub fn update_signal_mask(&self, new_mask: SigMask) -> Result<()> {
        if let Some(thread) = current_thread!().as_posix_thread() {
            thread.unregister_sigqueue_observer(&self.weak_self);
            let filter = SigEventsFilter::new(new_mask);
            thread.register_sigqueue_observer(self.weak_self.clone(), filter);
        }
        self.signals_mask.store(new_mask, Ordering::Relaxed);
        Ok(())
    }

    pub fn set_non_blocking(&self, non_blocking: bool) {
        self.non_blocking.store(non_blocking, Ordering::Relaxed);
    }

    fn is_non_blocking(&self) -> bool {
        self.non_blocking.load(Ordering::Relaxed)
    }

    /// Check current readable I/O events
    fn check_io_events(&self) -> IoEvents {
        let current = current_thread!();
        let Some(thread) = current.as_posix_thread() else {
            return IoEvents::empty();
        };

        let mask = self.signals_mask.load(Ordering::Relaxed);
        if thread.sig_pending().intersects(mask) {
            IoEvents::IN
        } else {
            IoEvents::empty()
        }
    }

    /// Attempt non-blocking read operation
    fn try_read(&self, writer: &mut VmWriter) -> Result<usize> {
        let current = current_thread!();
        let thread = current
            .as_posix_thread()
            .ok_or_else(|| Error::with_message(Errno::ESRCH, "Not a POSIX thread"))?;

        // Mask is inverted to get the signals that are not blocked
        let mask = !self.signals_mask.load(Ordering::Relaxed);
        let max_signals = writer.avail() / core::mem::size_of::<SignalfdSiginfo>();
        let mut count = 0;

        for _ in 0..max_signals {
            match thread.dequeue_signal(&mask) {
                Some(signal) => {
                    writer.write_val(&signal.to_signalfd_siginfo())?;
                    count += 1;
                    self.pollee.invalidate();
                }
                None => break,
            }
        }

        if count == 0 {
            return_errno!(Errno::EAGAIN);
        }
        Ok(count * core::mem::size_of::<SignalfdSiginfo>())
    }

    pub fn dentry(&self) -> &Dentry {
        &self.dentry
    }
}

impl Observer<SigEvents> for SignalFile {
    // TODO: Fix signal notifications.
    // Child processes do not inherit the parent's observer mechanism for signal event notifications.
    // `sys_poll` with blocking mode gets stuck if the signal is received after polling.
    fn on_events(&self, events: &SigEvents) {
        if self
            .signals_mask
            .load(Ordering::Relaxed)
            .contains(events.sig_num())
        {
            self.pollee.notify(IoEvents::IN);
        }
    }
}

impl Pollable for SignalFile {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        self.pollee
            .poll_with(mask, poller, || self.check_io_events())
    }
}

impl FileLike for SignalFile {
    fn read(&self, writer: &mut VmWriter) -> Result<usize> {
        if writer.avail() < core::mem::size_of::<SignalfdSiginfo>() {
            return_errno_with_message!(Errno::EINVAL, "Buffer too small for siginfo structure");
        }

        if self.is_non_blocking() {
            self.try_read(writer)
        } else {
            self.wait_events(IoEvents::IN, None, || self.try_read(writer))
        }
    }

    fn write(&self, _reader: &mut VmReader) -> Result<usize> {
        return_errno_with_message!(Errno::EBADF, "signalfd does not support write operations");
    }

    fn status_flags(&self) -> StatusFlags {
        if self.is_non_blocking() {
            StatusFlags::O_NONBLOCK
        } else {
            StatusFlags::empty()
        }
    }

    fn set_status_flags(&self, new_flags: StatusFlags) -> Result<()> {
        self.set_non_blocking(new_flags.contains(StatusFlags::O_NONBLOCK));
        Ok(())
    }

    fn metadata(&self) -> Metadata {
        self.dentry().metadata()
    }
}

impl Drop for SignalFile {
    // TODO: Fix signal notifications. See `on_events` method.
    fn drop(&mut self) {
        if let Some(thread) = current_thread!().as_posix_thread() {
            thread.unregister_sigqueue_observer(&self.weak_self);
        }
    }
}

#[repr(C)]
#[derive(Debug, Copy, Clone, Pod)]
struct SignalfdSiginfo {
    ssi_signo: u32,
    ssi_errno: i32,
    ssi_code: i32,
    ssi_pid: u32,
    ssi_uid: u32,
    ssi_fd: i32,
    ssi_tid: u32,
    ssi_band: u32,
    ssi_overrun: u32,
    ssi_trapno: u32,
    ssi_status: i32,
    ssi_int: i32,
    ssi_ptr: u64,
    ssi_utime: u64,
    ssi_stime: u64,
    ssi_addr: u64,
    _pad: [u8; 48],
}

trait ToSignalfdSiginfo {
    fn to_signalfd_siginfo(&self) -> SignalfdSiginfo;
}

impl ToSignalfdSiginfo for Box<dyn Signal> {
    fn to_signalfd_siginfo(&self) -> SignalfdSiginfo {
        let siginfo = self.to_info();
        SignalfdSiginfo {
            ssi_signo: siginfo.si_signo as _,
            ssi_errno: siginfo.si_errno,
            ssi_code: siginfo.si_code,
            ssi_pid: 0,
            ssi_uid: 0,
            ssi_fd: 0,
            ssi_tid: 0,
            ssi_band: 0,
            ssi_overrun: 0,
            ssi_trapno: 0,
            ssi_status: 0,
            ssi_int: 0,
            ssi_ptr: 0,
            ssi_utime: 0,
            ssi_stime: 0,
            ssi_addr: 0,
            _pad: [0; 48],
        }
    }
}
