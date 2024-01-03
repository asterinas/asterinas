// SPDX-License-Identifier: MPL-2.0

use crate::fs::{
    file_table::FileDescripter, fs_resolver::FsPath, inode_handle::InodeHandle, utils::InodeType,
};
use crate::log_syscall_entry;
use crate::prelude::*;
use crate::syscall::constants::MAX_FILENAME_LEN;
use crate::util::read_cstring_from_user;

use super::SyscallReturn;
use super::{SYS_CHDIR, SYS_FCHDIR};

pub fn sys_chdir(pathname_addr: Vaddr) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_CHDIR);
    let pathname = read_cstring_from_user(pathname_addr, MAX_FILENAME_LEN)?;
    debug!("pathname = {:?}", pathname);

    let current = current!();
    let mut fs = current.fs().write();
    let dentry = {
        let pathname = pathname.to_string_lossy();
        if pathname.is_empty() {
            return_errno_with_message!(Errno::ENOENT, "path is empty");
        }
        let fs_path = FsPath::try_from(pathname.as_ref())?;
        fs.lookup(&fs_path)?
    };
    if dentry.inode_type() != InodeType::Dir {
        return_errno_with_message!(Errno::ENOTDIR, "must be directory");
    }
    fs.set_cwd(dentry);
    Ok(SyscallReturn::Return(0))
}

pub fn sys_fchdir(fd: FileDescripter) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_FCHDIR);
    debug!("fd = {}", fd);

    let current = current!();
    let dentry = {
        let file_table = current.file_table().lock();
        let file = file_table.get_file(fd)?;
        let inode_handle = file
            .downcast_ref::<InodeHandle>()
            .ok_or(Error::with_message(Errno::EBADF, "not inode"))?;
        inode_handle.dentry().clone()
    };
    if dentry.inode_type() != InodeType::Dir {
        return_errno_with_message!(Errno::ENOTDIR, "must be directory");
    }
    current.fs().write().set_cwd(dentry);
    Ok(SyscallReturn::Return(0))
}
