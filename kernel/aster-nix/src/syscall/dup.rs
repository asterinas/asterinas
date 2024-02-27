// SPDX-License-Identifier: MPL-2.0

use super::{SyscallReturn, SYS_DUP, SYS_DUP2};
use crate::{fs::file_table::FileDescripter, log_syscall_entry, prelude::*};

pub fn sys_dup(old_fd: FileDescripter) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_DUP);
    debug!("old_fd = {}", old_fd);

    let current = current!();
    let mut file_table = current.file_table().lock();
    let file = file_table.get_file(old_fd)?.clone();
    let new_fd = file_table.insert(file);
    Ok(SyscallReturn::Return(new_fd as _))
}

pub fn sys_dup2(old_fd: FileDescripter, new_fd: FileDescripter) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_DUP2);
    debug!("old_fd = {}, new_fd = {}", old_fd, new_fd);

    let current = current!();
    let mut file_table = current.file_table().lock();
    let file = file_table.get_file(old_fd)?.clone();
    if old_fd != new_fd {
        if let Some(old_file) = file_table.insert_at(new_fd, file) {
            // If the file descriptor `new_fd` was previously open, close it silently.
            let _ = old_file.clean_for_close();
        }
    }
    Ok(SyscallReturn::Return(new_fd as _))
}
