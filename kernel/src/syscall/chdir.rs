// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::{
        file_table::{get_file_fast, FileDesc},
        fs_resolver::FsPath,
        utils::InodeType,
    },
    prelude::*,
    syscall::constants::MAX_FILENAME_LEN,
};

pub fn sys_chdir(path_ptr: Vaddr, ctx: &Context) -> Result<SyscallReturn> {
    let path = ctx.user_space().read_cstring(path_ptr, MAX_FILENAME_LEN)?;
    debug!("path = {:?}", path);

    let mut fs = ctx.posix_thread.fs().resolver().write();
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
        let mut file_table = ctx.thread_local.borrow_file_table_mut();
        let file = get_file_fast!(&mut file_table, fd);
        file.as_inode_or_err()?.dentry().clone()
    };
    if dentry.type_() != InodeType::Dir {
        return_errno_with_message!(Errno::ENOTDIR, "must be directory");
    }
    ctx.posix_thread.fs().resolver().write().set_cwd(dentry);
    Ok(SyscallReturn::Return(0))
}
