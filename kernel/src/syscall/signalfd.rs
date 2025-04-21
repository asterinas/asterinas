// SPDX-License-Identifier: MPL-2.0

//! signalfd implementation for Linux compatibility
//!
//! The signalfd mechanism allows receiving signals via file descriptor,
//! enabling better integration with event loops.
//! See https://man7.org/linux/man-pages/man2/signalfd.2.html

use core::sync::atomic::Ordering;

use bitflags::bitflags;

use super::SyscallReturn;
use crate::{
    fs::{
        file_table::{get_file_fast, FdFlags, FileDesc},
        pseudo::SignalFile,
        utils::{CreationFlags, StatusFlags},
    },
    prelude::*,
    process::signal::{
        constants::{SIGKILL, SIGSTOP},
        sig_mask::{AtomicSigMask, SigMask},
        SigEventsFilter,
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

    if sizemask != core::mem::size_of::<SigMask>() {
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
    let atomic_mask = AtomicSigMask::new(mask);
    let signal_file = SignalFile::new(atomic_mask, non_blocking);

    register_observer(ctx, &signal_file, mask)?;

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

fn register_observer(ctx: &Context, signal_file: &Arc<SignalFile>, mask: SigMask) -> Result<()> {
    let filter = SigEventsFilter::new(mask);

    ctx.posix_thread
        .register_sigqueue_observer(signal_file.observer_ref(), filter);

    Ok(())
}

bitflags! {
    /// Signal file descriptor creation flags
    struct SignalFileFlags: u32 {
        const O_CLOEXEC = CreationFlags::O_CLOEXEC.bits();
        const O_NONBLOCK = StatusFlags::O_NONBLOCK.bits();
    }
}
