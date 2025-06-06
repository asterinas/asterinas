// SPDX-License-Identifier: MPL-2.0

use ostd::sync::RwArc;

use super::SyscallReturn;
use crate::{
    fs::file_table::{FdFlags, FileDesc, FileTable},
    prelude::*,
};

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

pub fn sys_close_range(first: u32, last: u32, flags: u32, ctx: &Context) -> Result<SyscallReturn> {
    const CLOSE_RANGE_UNSHARE: u32 = 1 << 1;
    const CLOSE_RANGE_CLOEXEC: u32 = 1 << 2;

    debug!("first = {}, last = {}, flags = {}", first, last, flags);

    if last < first {
        return_errno!(Errno::EINVAL);
    }

    if flags & !(CLOSE_RANGE_CLOEXEC | CLOSE_RANGE_UNSHARE) != 0 {
        return_errno!(Errno::EINVAL);
    }

    let original_table = ctx.thread_local.borrow_file_table().unwrap().clone();

    let file_table = if flags & CLOSE_RANGE_UNSHARE != 0 {
        let new_table = deep_clone_file_table(&original_table);
        ctx.thread_local.set_file_table(Some(new_table.clone()));
        new_table
    } else {
        original_table
    };

    let mut files_to_drop = Vec::new();

    {
        let mut file_table_locked = file_table.write();

        let table_len = file_table_locked.len() as u32;
        if table_len == 0 || first >= table_len {
            return Ok(SyscallReturn::Return(0));
        }
        let actual_last = last.min(table_len - 1);

        for fd in first..=actual_last {
            let fd = fd as FileDesc;

            if flags & CLOSE_RANGE_CLOEXEC != 0 {
                if let Ok(entry) = file_table_locked.get_entry_mut(fd) {
                    entry.set_flags(entry.flags() | FdFlags::CLOEXEC);
                }
            } else {
                if let Some(file) = file_table_locked.close_file(fd) {
                    files_to_drop.push(file);
                }
            }
        }
    }

    drop(files_to_drop);

    Ok(SyscallReturn::Return(0))
}

fn deep_clone_file_table(original: &RwArc<FileTable>) -> RwArc<FileTable> {
    let original_guard = original.read();
    let mut new_table = FileTable::new();

    for (fd, entry) in original_guard.entries() {
        new_table.put_entry_at(fd, entry.clone());
    }

    RwArc::new(new_table)
}
