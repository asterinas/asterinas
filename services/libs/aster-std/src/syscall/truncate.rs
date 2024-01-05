// SPDX-License-Identifier: MPL-2.0

use crate::fs::{
    file_table::FileDescripter,
    fs_resolver::{FsPath, AT_FDCWD},
    utils::PATH_MAX,
};
use crate::log_syscall_entry;
use crate::prelude::*;
use crate::process::ResourceType;
use crate::util::read_cstring_from_user;

use super::SyscallReturn;
use super::{SYS_FTRUNCATE, SYS_TRUNCATE};

pub fn sys_ftruncate(fd: FileDescripter, len: isize) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_FTRUNCATE);
    debug!("fd = {}, lentgh = {}", fd, len);

    check_length(len)?;

    let current = current!();
    let file_table = current.file_table().lock();
    let file = file_table.get_file(fd)?;
    file.resize(len as usize)?;
    Ok(SyscallReturn::Return(0))
}

pub fn sys_truncate(path_ptr: Vaddr, len: isize) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_TRUNCATE);
    let path = read_cstring_from_user(path_ptr, PATH_MAX)?;
    debug!("path = {:?}, length = {}", path, len);

    check_length(len)?;

    let current = current!();
    let dentry = {
        let path = path.to_string_lossy();
        if path.is_empty() {
            return_errno_with_message!(Errno::ENOENT, "path is empty");
        }
        let fs_path = FsPath::new(AT_FDCWD, path.as_ref())?;
        current.fs().read().lookup(&fs_path)?
    };
    dentry.set_inode_size(len as usize)?;
    Ok(SyscallReturn::Return(0))
}

#[inline]
fn check_length(len: isize) -> Result<()> {
    if len < 0 {
        return_errno_with_message!(Errno::EINVAL, "length is negative");
    }

    let max_file_size = {
        let current = current!();
        let resource_limits = current.resource_limits().lock();
        resource_limits
            .get_rlimit(ResourceType::RLIMIT_FSIZE)
            .get_cur() as usize
    };
    if len as usize > max_file_size {
        return_errno_with_message!(Errno::EFBIG, "length is larger than the maximum file size");
    }
    Ok(())
}
