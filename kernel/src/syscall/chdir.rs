// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::{file_table::FileDesc, fs_resolver::FsPath, inode_handle::InodeHandle, utils::InodeType},
    prelude::*,
    syscall::constants::MAX_FILENAME_LEN,
};

pub fn sys_chdir(path_ptr: Vaddr, ctx: &Context) -> Result<SyscallReturn> {
    let path = ctx.user_space().read_cstring(path_ptr, MAX_FILENAME_LEN)?;
    debug!("path = {:?}", path);

    let mut fs = ctx.process.fs().write();
    let dentry = {
        let path = path.to_string_lossy();
        if path.is_empty() {
            return_errno_with_message!(Errno::ENOENT, "path is empty");
        }
        let fs_path = FsPath::try_from(path.as_ref())?;
        fs.lookup(&fs_path)?
    };
    if dentry.type_() != InodeType::Dir {
        return_errno_with_message!(Errno::ENOTDIR, "must be directory");
    }
    fs.set_cwd(dentry);
    Ok(SyscallReturn::Return(0))
}

pub fn sys_fchdir(fd: FileDesc, ctx: &Context) -> Result<SyscallReturn> {
    debug!("fd = {}", fd);

    let dentry = {
        let file_table = ctx.process.file_table().lock();
        let file = file_table.get_file(fd)?;
        let inode_handle = file
            .downcast_ref::<InodeHandle>()
            .ok_or(Error::with_message(Errno::EBADF, "not inode"))?;
        inode_handle.dentry().clone()
    };
    if dentry.type_() != InodeType::Dir {
        return_errno_with_message!(Errno::ENOTDIR, "must be directory");
    }
    ctx.process.fs().write().set_cwd(dentry);
    Ok(SyscallReturn::Return(0))
}
