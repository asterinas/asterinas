// SPDX-License-Identifier: MPL-2.0

//! signalfd implementation for Linux compatibility
//!
//! The signalfd mechanism allows receiving signals via file descriptor,
//! enabling better integration with event loops.
//! See <https://man7.org/linux/man-pages/man2/signalfd.2.html>.

use core::{
    fmt::Display,
    sync::atomic::{AtomicBool, Ordering},
};

use bitflags::bitflags;
use ostd::mm::VmIo;

use super::SyscallReturn;
use crate::{
    events::IoEvents,
    fs::{
        file_handle::FileLike,
        file_table::{FdFlags, FileDesc, get_file_fast},
        path::Path,
        pseudofs::AnonInodeFs,
        utils::{CreationFlags, StatusFlags},
    },
    prelude::*,
    process::{
        posix_thread::{AsPosixThread, PosixThread},
        signal::{
            HandlePendingSignal, PollHandle, Pollable, Poller,
            constants::{SIGKILL, SIGSTOP},
            sig_mask::{AtomicSigMask, SigMask},
            signals::Signal,
        },
    },
};

/// Creates a new signalfd or updates an existing one according to the given mask
pub fn sys_signalfd(
    fd: FileDesc,
    mask_ptr: Vaddr,
    sizemask: usize,
    ctx: &Context,
) -> Result<SyscallReturn> {
    sys_signalfd4(fd, mask_ptr, sizemask, 0, ctx)
}

/// Creates a new signalfd or updates an existing one according to the given mask and flags
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

    if sizemask != size_of::<SigMask>() {
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
    mask: SigMask,
    non_blocking: bool,
    fd_flags: FdFlags,
) -> Result<FileDesc> {
    let signal_file = {
        let atomic_mask = AtomicSigMask::new(mask);
        Arc::new(SignalFile::new(atomic_mask, non_blocking))
    };

    let file_table = ctx.thread_local.borrow_file_table();
    let fd = file_table.unwrap().write().insert(signal_file, fd_flags);
    Ok(fd)
}

fn update_existing_signalfd(
    ctx: &Context,
    fd: FileDesc,
    new_mask: SigMask,
    non_blocking: bool,
) -> Result<FileDesc> {
    let mut file_table = ctx.thread_local.borrow_file_table_mut();
    let file = get_file_fast!(&mut file_table, fd);
    let signal_file = file
        .downcast_ref::<SignalFile>()
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "File descriptor is not a signalfd"))?;

    if signal_file.mask().load(Ordering::Relaxed) != new_mask {
        signal_file.update_signal_mask(new_mask)?;
    }
    signal_file.set_non_blocking(non_blocking);
    Ok(fd)
}

bitflags! {
    /// Signal file descriptor creation flags
    struct SignalFileFlags: u32 {
        const O_CLOEXEC = CreationFlags::O_CLOEXEC.bits();
        const O_NONBLOCK = StatusFlags::O_NONBLOCK.bits();
    }
}

/// Signal file implementation
///
/// Represents a file that can be used to receive signals
/// as readable events.
struct SignalFile {
    /// Atomic signal mask for filtering signals
    signals_mask: AtomicSigMask,
    /// Non-blocking mode flag
    non_blocking: AtomicBool,
    /// The pseudo path associated with this signalfd file.
    pseudo_path: Path,
}

impl SignalFile {
    /// Create a new signalfd instance
    fn new(mask: AtomicSigMask, non_blocking: bool) -> Self {
        let pseudo_path = AnonInodeFs::new_path(|_| "anon_inode:[signalfd]".to_string());

        Self {
            signals_mask: mask,
            non_blocking: AtomicBool::new(non_blocking),
            pseudo_path,
        }
    }

    fn mask(&self) -> &AtomicSigMask {
        &self.signals_mask
    }

    fn update_signal_mask(&self, new_mask: SigMask) -> Result<()> {
        self.signals_mask.store(new_mask, Ordering::Relaxed);
        Ok(())
    }

    fn set_non_blocking(&self, non_blocking: bool) {
        self.non_blocking.store(non_blocking, Ordering::Relaxed);
    }

    fn is_non_blocking(&self) -> bool {
        self.non_blocking.load(Ordering::Relaxed)
    }

    /// Check current readable I/O events
    fn check_io_events(&self, posix_thread: &PosixThread) -> IoEvents {
        let mask = self.signals_mask.load(Ordering::Relaxed);
        if posix_thread.pending_signals().intersects(mask) {
            IoEvents::IN
        } else {
            IoEvents::empty()
        }
    }

    /// Attempt non-blocking read operation
    fn try_read(&self, writer: &mut VmWriter, thread: &PosixThread) -> Result<usize> {
        // Mask is inverted to get the signals that are not blocked
        let mask = !self.signals_mask.load(Ordering::Relaxed);
        let max_signals = writer.avail() / size_of::<SignalfdSiginfo>();
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

        if count == 0 {
            return_errno!(Errno::EAGAIN);
        }
        Ok(count * size_of::<SignalfdSiginfo>())
    }
}

impl Pollable for SignalFile {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        let current = current_thread!();
        let Some(posix_thread) = current.as_posix_thread() else {
            return IoEvents::empty();
        };

        if let Some(poller) = poller {
            posix_thread.register_signalfd_poller(poller, mask);
        }

        self.check_io_events(posix_thread) & mask
    }
}

impl FileLike for SignalFile {
    fn read(&self, writer: &mut VmWriter) -> Result<usize> {
        if writer.avail() < size_of::<SignalfdSiginfo>() {
            return_errno_with_message!(Errno::EINVAL, "Buffer too small for siginfo structure");
        }

        let thread = current_thread!();
        let posix_thread = thread
            .as_posix_thread()
            .ok_or_else(|| Error::with_message(Errno::ESRCH, "Not a POSIX thread"))?;

        // Fast path: There are already pending signals or the signalfd is non-blocking.
        // So we don't need to create and register the poller.
        match self.try_read(writer, posix_thread) {
            Err(e) if e.error() == Errno::EAGAIN && !self.is_non_blocking() => {}
            res => return res,
        }

        // Slow path
        let mut poller = Poller::new(None);
        posix_thread.register_signalfd_poller(poller.as_handle_mut(), IoEvents::IN);

        loop {
            match self.try_read(writer, posix_thread) {
                Err(e) if e.error() == Errno::EAGAIN => poller.wait()?,
                res => return res,
            }
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

    fn path(&self) -> &Path {
        &self.pseudo_path
    }

    fn dump_proc_fdinfo(self: Arc<Self>, fd_flags: FdFlags) -> Box<dyn Display> {
        struct FdInfo {
            flags: u32,
            sigmask: u64,
        }

        impl Display for FdInfo {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                writeln!(f, "pos:\t{}", 0)?;
                writeln!(f, "flags:\t0{:o}", self.flags)?;
                writeln!(f, "mnt_id:\t{}", AnonInodeFs::mount_node().id())?;
                writeln!(f, "ino:\t{}", AnonInodeFs::shared_inode().ino())?;
                writeln!(f, "sigmask:\t{:016x}", self.sigmask)
            }
        }

        let mut flags = self.status_flags().bits() | self.access_mode() as u32;
        if fd_flags.contains(FdFlags::CLOEXEC) {
            flags |= CreationFlags::O_CLOEXEC.bits();
        }

        Box::new(FdInfo {
            flags,
            sigmask: self.mask().load(Ordering::Relaxed).into(),
        })
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
