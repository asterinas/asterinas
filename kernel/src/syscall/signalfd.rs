// SPDX-License-Identifier: MPL-2.0

//! signalfd implementation for Linux compatibility
//!
//! The signalfd mechanism allows receiving signals via file descriptor,
//! enabling better integration with event loops.
//! See https://man7.org/linux/man-pages/man2/signalfd.2.html

use core::sync::atomic::{AtomicBool, Ordering};

use bitflags::bitflags;
use ostd::{sync::WaitQueue, task::Task};

use super::SyscallReturn;
use crate::{
    events::{IoEvents, Observer},
    fs::{
        file_handle::FileLike,
        file_table::{get_file_fast, FdFlags, FileDesc},
        utils::{CreationFlags, InodeMode, InodeType, Metadata, StatusFlags},
    },
    prelude::*,
    process::{
        posix_thread::AsPosixThread,
        signal::{
            constants::{SIGKILL, SIGSTOP},
            sig_mask::{AtomicSigMask, SigMask, SigSet},
            signals::Signal,
            PollHandle, Pollable, Pollee, SigEvents, SigEventsFilter,
        },
        Gid, Uid,
    },
    time::clocks::RealTimeClock,
};

/// Create new signalfd file descriptor
pub fn sys_signalfd(
    fd: FileDesc,
    mask_ptr: Vaddr,
    sizemask: usize,
    ctx: &Context,
) -> Result<SyscallReturn> {
    sys_signalfd4(fd, mask_ptr, sizemask, 0, ctx)
}

/// Create/update signalfd with flags
pub fn sys_signalfd4(
    fd: FileDesc,
    mask_ptr: Vaddr,
    sizemask: usize,
    flags: i32,
    ctx: &Context,
) -> Result<SyscallReturn> {
    debug!(
        "fd = {}, mask = {:x}, sizemask = {}, flags = {}",
        fd, mask_ptr, sizemask, flags
    );

    if sizemask != core::mem::size_of::<SigSet>() {
        return Err(Error::with_message(Errno::EINVAL, "invalid mask size"));
    }

    let mut mask = ctx.user_space().read_val::<SigMask>(mask_ptr)?;
    mask -= SIGKILL;
    mask -= SIGSTOP;

    let flags = SignalFileFlags::from_bits(flags as u32)
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "invalid flags"))?;

    let fd_flags = if flags.contains(SignalFileFlags::O_CLOEXEC) {
        FdFlags::CLOEXEC
    } else {
        FdFlags::empty()
    };

    let non_blocking = flags.contains(SignalFileFlags::O_NONBLOCK);

    let new_fd = if fd == -1 {
        create_new_signalfd(ctx, mask, non_blocking, fd_flags)?
    } else {
        update_existing_signalfd(ctx, fd, mask, non_blocking)?
    };

    Ok(SyscallReturn::Return(new_fd as _))
}

fn create_new_signalfd(
    ctx: &Context,
    mask: SigSet,
    non_blocking: bool,
    fd_flags: FdFlags,
) -> Result<FileDesc> {
    let atomic_mask = AtomicSigMask::new(mask);
    let task_ref = Arc::downgrade(&ctx.thread.task());
    let signal_file = Arc::new(SignalFile::new(task_ref, atomic_mask, non_blocking));

    register_observer(ctx, &signal_file, mask)?;

    let file_table = ctx.thread_local.file_table().borrow_mut();
    let fd = file_table.write().insert(signal_file, fd_flags);
    Ok(fd)
}

fn update_existing_signalfd(
    ctx: &Context,
    fd: FileDesc,
    new_mask: SigSet,
    non_blocking: bool,
) -> Result<FileDesc> {
    let mut file_table = ctx.thread_local.file_table().borrow_mut();
    let file = get_file_fast!(&mut file_table, fd);
    let signal_file = file
        .downcast_ref::<SignalFile>()
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "File descriptor is not a signalfd"))?;

    if signal_file.mask().load(Ordering::Relaxed) != new_mask {
        update_signal_mask(signal_file, new_mask)?;
    }
    signal_file.set_non_blocking(non_blocking);
    Ok(fd)
}

fn register_observer(ctx: &Context, signal_file: &Arc<SignalFile>, mask: SigSet) -> Result<()> {
    let observer = Arc::downgrade(signal_file) as Weak<dyn Observer<SigEvents>>;
    let filter = SigEventsFilter::new(mask);

    ctx.posix_thread
        .register_sigqueue_observer(observer.clone(), filter);

    *signal_file.observer_lock().lock() = Some(observer);
    Ok(())
}

fn update_signal_mask(signal_file: &SignalFile, new_mask: SigSet) -> Result<()> {
    let old_observer = signal_file.observer_lock().lock().take();
    if let Some(task) = signal_file.task().upgrade() {
        if let Some(thread) = task.as_posix_thread() {
            if let Some(old_observer) = old_observer {
                thread.unregister_sigqueue_observer(&old_observer);
                let filter = SigEventsFilter::new(new_mask);
                thread.register_sigqueue_observer(old_observer.clone(), filter);
                *signal_file.observer_lock().lock() = Some(old_observer);
            }
        }
    }
    signal_file.mask().store(new_mask, Ordering::Relaxed);

    Ok(())
}

bitflags! {
    /// Signal file descriptor creation flags
    struct SignalFileFlags: u32 {
        const O_CLOEXEC = CreationFlags::O_CLOEXEC.bits();
        const O_NONBLOCK = StatusFlags::O_NONBLOCK.bits();
    }
}

/// Signal file descriptor implementation
///
/// Represents a file descriptor that can be used to receive signals
/// as readable events.
struct SignalFile {
    /// Weak reference to associated task
    task_ref: Weak<Task>,
    /// Atomic signal mask for filtering signals
    signals_mask: AtomicSigMask,
    /// I/O event notifier
    pollee: Pollee,
    /// Non-blocking mode flag
    non_blocking: AtomicBool,
    /// Observer registration handle
    observer_handle: Mutex<Option<Weak<dyn Observer<SigEvents>>>>,
    /// Wait queue for blocking I/O
    wait_queue: WaitQueue,
}

impl SignalFile {
    /// Create a new signalfd instance
    fn new(task_ref: Weak<Task>, mask: AtomicSigMask, non_blocking: bool) -> Self {
        Self {
            task_ref,
            signals_mask: mask,
            pollee: Pollee::new(),
            non_blocking: AtomicBool::new(non_blocking),
            observer_handle: Mutex::new(None),
            wait_queue: WaitQueue::new(),
        }
    }

    fn mask(&self) -> &AtomicSigMask {
        &self.signals_mask
    }

    fn observer_lock(&self) -> &Mutex<Option<Weak<dyn Observer<SigEvents>>>> {
        &self.observer_handle
    }

    fn task(&self) -> &Weak<Task> {
        &self.task_ref
    }

    fn set_non_blocking(&self, non_blocking: bool) {
        self.non_blocking.store(non_blocking, Ordering::Relaxed);
    }

    fn is_non_blocking(&self) -> bool {
        self.non_blocking.load(Ordering::Relaxed)
    }

    /// Check current readable I/O events
    fn check_io_events(&self) -> IoEvents {
        if let Some(task) = self.task().upgrade() {
            let thread = match task.as_posix_thread() {
                Some(t) => t,
                None => return IoEvents::empty(),
            };
            let mask = self.signals_mask.load(Ordering::Relaxed);
            if thread.sig_pending().intersects(mask) {
                IoEvents::IN
            } else {
                IoEvents::empty()
            }
        } else {
            IoEvents::empty()
        }
    }

    /// Attempt non-blocking read operation
    fn try_read(&self, writer: &mut VmWriter) -> Result<usize> {
        let buffer_size = writer.avail();
        if buffer_size % core::mem::size_of::<SignalfdSiginfo>() != 0 {
            return_errno_with_message!(
                Errno::EINVAL,
                "Buffer size must be multiple of SignalfdSiginfo size"
            );
        }

        let task = self
            .task()
            .upgrade()
            .ok_or_else(|| Error::with_message(Errno::ESRCH, "Associated task no longer exists"))?;

        let thread = task
            .as_posix_thread()
            .ok_or_else(|| Error::with_message(Errno::ESRCH, "Not a POSIX thread"))?;

        // Mask is inverted to get the signals that are not blocked
        let mask = !self.signals_mask.load(Ordering::Relaxed);
        let max_signals = buffer_size / core::mem::size_of::<SignalfdSiginfo>();
        let mut count = 0;

        for _ in 0..max_signals {
            match thread.dequeue_signal(&mask) {
                Some(signal) => {
                    writer.write_val(&signal.to_signalfd_siginfo())?;
                    count += 1;
                }
                None => break,
            }
        }

        match count {
            0 => Err(Error::with_message(
                Errno::EAGAIN,
                "No signals available in non-blocking mode",
            )),
            n => Ok(n * core::mem::size_of::<SignalfdSiginfo>()),
        }
    }
}

impl Observer<SigEvents> for SignalFile {
    fn on_events(&self, events: &SigEvents) {
        if self
            .signals_mask
            .load(Ordering::Relaxed)
            .contains(events.sig_num())
        {
            self.pollee.notify(IoEvents::IN);
            self.wait_queue.wake_all();
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
            self.wait_queue.wait_until(|| Some(self.try_read(writer)))
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
        let now = RealTimeClock::get().read_time();
        Metadata {
            dev: 0,
            ino: 0,
            size: 0,
            blk_size: 0,
            blocks: 0,
            atime: now,
            mtime: now,
            ctime: now,
            type_: InodeType::NamedPipe,
            mode: InodeMode::from_bits_truncate(0o400),
            nlinks: 1,
            uid: Uid::new_root(),
            gid: Gid::new_root(),
            rdev: 0,
        }
    }
}

impl Drop for SignalFile {
    fn drop(&mut self) {
        if let Some(observer) = self.observer_handle.lock().take() {
            if let Some(task) = self.task().upgrade() {
                if let Some(thread) = task.as_posix_thread() {
                    thread.unregister_sigqueue_observer(&observer);
                }
            }
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

trait SignalToSiginfo {
    fn to_signalfd_siginfo(&self) -> SignalfdSiginfo;
}

impl SignalToSiginfo for Box<dyn Signal> {
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
