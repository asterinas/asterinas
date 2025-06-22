// SPDX-License-Identifier: MPL-2.0

use bitflags::bitflags;
use ostd::sync::RwArc;

use super::SyscallReturn;
use crate::{
    fs::file_table::{FdFlags, FileDesc},
    prelude::*,
};

bitflags! {
    struct CloseRangeFlags: u32 {
        const UNSHARE = 1 << 1;
        const CLOEXEC = 1 << 2;
    }
}

pub fn sys_close(fd: FileDesc, ctx: &Context) -> Result<SyscallReturn> {
    debug!("fd = {}", fd);

    let file = {
        let file_table = ctx.thread_local.borrow_file_table();
        let mut file_table_locked = file_table.unwrap().write();
        let _ = file_table_locked.get_file(fd)?;
        file_table_locked.close_file(fd).unwrap()
    };

    // Cleanup work needs to be done in the `Drop` impl.
    //
    // We don't mind the races between closing the file descriptor and using the file descriptor,
    // because such races are explicitly allowed in the man pages. See the "Multithreaded processes
    // and close()" section in <https://man7.org/linux/man-pages/man2/close.2.html>.
    drop(file);

    // Linux has error codes for the close() system call for diagnostic and remedial purposes, but
    // only for a small subset of file systems such as NFS. We currently have no support for such
    // file systems, so it's fine to just return zero.
    //
    // For details, see the discussion at <https://github.com/asterinas/asterinas/issues/506> and
    // the "Dealing with error returns from close()" section at
    // <https://man7.org/linux/man-pages/man2/close.2.html>.
    Ok(SyscallReturn::Return(0))
}

pub fn sys_close_range(
    first: u32,
    last: u32,
    raw_flags: u32,
    ctx: &Context,
) -> Result<SyscallReturn> {
    debug!("first = {}, last = {}, flags = {}", first, last, raw_flags);

    if last < first {
        return_errno!(Errno::EINVAL);
    }

    let flags = CloseRangeFlags::from_bits(raw_flags).ok_or_else(|| Error::new(Errno::EINVAL))?;

    let original_table = ctx.thread_local.borrow_file_table().unwrap().clone();

    let file_table = if flags.contains(CloseRangeFlags::UNSHARE) {
        let new_table = RwArc::new(original_table.get_cloned());
        let _ = ctx
            .thread_local
            .borrow_file_table_mut()
            .replace(Some(new_table.clone()));
        new_table
    } else {
        original_table
    };

    let mut files_to_drop = Vec::new();

    {
        let mut file_table_locked = file_table.write();

        let table_len = file_table_locked.len() as u32;
        if first >= table_len {
            return Ok(SyscallReturn::Return(0));
        }
        let actual_last = last.min(table_len - 1);

        for fd in first..=actual_last {
            let fd = fd as FileDesc;

            if flags.contains(CloseRangeFlags::CLOEXEC) {
                if let Ok(entry) = file_table_locked.get_entry_mut(fd) {
                    entry.set_flags(entry.flags() | FdFlags::CLOEXEC);
                }
            } else if let Some(file) = file_table_locked.close_file(fd) {
                files_to_drop.push(file);
            }
        }
    }

    drop(files_to_drop);

    Ok(SyscallReturn::Return(0))
}
