// SPDX-License-Identifier: MPL-2.0

use super::{SyscallReturn, SYS_FSYNC};
use crate::{
    fs::{file_table::FileDescripter, inode_handle::InodeHandle},
    log_syscall_entry,
    prelude::*,
};

pub fn sys_fsync(fd: FileDescripter) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_FSYNC);
    debug!("fd = {}", fd);

    let dentry = {
        let current = current!();
        let file_table = current.file_table().lock();
        let file = file_table.get_file(fd)?;
        let inode_handle = file
            .downcast_ref::<InodeHandle>()
            .ok_or(Error::with_message(Errno::EINVAL, "not inode"))?;
        inode_handle.dentry().clone()
    };
    dentry.sync()?;
    Ok(SyscallReturn::Return(0))
}
