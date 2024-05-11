// SPDX-License-Identifier: MPL-2.0

use super::{SyscallReturn, SYS_CHDIR, SYS_FCHDIR};
use crate::{
    fs::{file_table::FileDesc, fs_resolver::FsPath, inode_handle::InodeHandle, utils::InodeType},
    log_syscall_entry,
    prelude::*,
    syscall::constants::MAX_FILENAME_LEN,
    util::read_cstring_from_user,
};

pub fn sys_chdir(path_ptr: Vaddr) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_CHDIR);
    let path = read_cstring_from_user(path_ptr, MAX_FILENAME_LEN)?;
    debug!("path = {:?}", path);

    let current = current!();
    let mut fs = current.fs().write();
    let dentrymnt = {
        let path = path.to_string_lossy();
        if path.is_empty() {
            return_errno_with_message!(Errno::ENOENT, "path is empty");
        }
        let fs_path = FsPath::try_from(path.as_ref())?;
        fs.lookup(&fs_path)?
    };
    if dentrymnt.type_() != InodeType::Dir {
        return_errno_with_message!(Errno::ENOTDIR, "must be directory");
    }
    fs.set_cwd(dentrymnt);
    Ok(SyscallReturn::Return(0))
}

pub fn sys_fchdir(fd: FileDesc) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_FCHDIR);
    debug!("fd = {}", fd);

    let current = current!();
    let dentrymnt = {
        let file_table = current.file_table().lock();
        let file = file_table.get_file(fd)?;
        let inode_handle = file
            .downcast_ref::<InodeHandle>()
            .ok_or(Error::with_message(Errno::EBADF, "not inode"))?;
        inode_handle.dentrymnt().clone()
    };
    if dentrymnt.type_() != InodeType::Dir {
        return_errno_with_message!(Errno::ENOTDIR, "must be directory");
    }
    current.fs().write().set_cwd(dentrymnt);
    Ok(SyscallReturn::Return(0))
}
