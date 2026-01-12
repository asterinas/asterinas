// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::{
        file_table::FileDesc,
        path::{AT_FDCWD, FsPath, SplitPath},
    },
    prelude::*,
    syscall::constants::MAX_FILENAME_LEN,
};

pub fn sys_rmdir(path_addr: Vaddr, ctx: &Context) -> Result<SyscallReturn> {
    self::sys_rmdirat(AT_FDCWD, path_addr, ctx)
}

pub(super) fn sys_rmdirat(
    dirfd: FileDesc,
    path_addr: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let path_name = ctx.user_space().read_cstring(path_addr, MAX_FILENAME_LEN)?;
    debug!("dirfd = {}, path_addr = {:?}", dirfd, path_addr);

    let path_name = path_name.to_string_lossy();
    let (dir_path, name) = {
        let (parent_path_name, target_name) = path_name.split_dirname_and_basename()?;
        let fs_path = FsPath::from_fd_and_path(dirfd, parent_path_name)?;
        (
            ctx.thread_local
                .borrow_fs()
                .resolver()
                .read()
                .lookup(&fs_path)?,
            target_name,
        )
    };

    dir_path.rmdir(name)?;
    Ok(SyscallReturn::Return(0))
}
