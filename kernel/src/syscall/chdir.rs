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
    let path_name = ctx.user_space().read_cstring(path_ptr, MAX_FILENAME_LEN)?;
    debug!("path = {:?}", path_name);

    let fs_ref = ctx.thread_local.borrow_fs();
    let mut fs = fs_ref.resolver().write();
    let path = {
        let path_name = path_name.to_string_lossy();
        if path_name.is_empty() {
            return_errno_with_message!(Errno::ENOENT, "path is empty");
        }
        let fs_path = FsPath::try_from(path_name.as_ref())?;
        fs.lookup(&fs_path)?
    };
    if path.type_() != InodeType::Dir {
        return_errno_with_message!(Errno::ENOTDIR, "must be directory");
    }
    fs.set_cwd(path);
    Ok(SyscallReturn::Return(0))
}

pub fn sys_fchdir(fd: FileDesc, ctx: &Context) -> Result<SyscallReturn> {
    debug!("fd = {}", fd);

    let path = {
        let mut file_table = ctx.thread_local.borrow_file_table_mut();
        let file = get_file_fast!(&mut file_table, fd);
        file.as_inode_or_err()?.path().clone()
    };
    if path.type_() != InodeType::Dir {
        return_errno_with_message!(Errno::ENOTDIR, "must be directory");
    }
    let fs_ref = ctx.thread_local.borrow_fs();
    fs_ref.resolver().write().set_cwd(path);
    Ok(SyscallReturn::Return(0))
}
