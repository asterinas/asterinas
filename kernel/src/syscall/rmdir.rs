// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::{
        file_table::FileDesc,
        fs_resolver::{split_path, FsPath, AT_FDCWD},
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
    if path_name.is_empty() {
        return_errno_with_message!(Errno::ENOENT, "path is empty");
    }
    if path_name.trim_end_matches('/').is_empty() {
        return_errno_with_message!(Errno::EBUSY, "is root directory");
    }

    let (dir_path, name) = {
        let (parent_path_name, target_name) = split_path(&path_name);
        let fs_path = FsPath::new(dirfd, parent_path_name)?;
        (
            ctx.thread_local
                .borrow_fs()
                .resolver()
                .read()
                .lookup(&fs_path)?,
            target_name,
        )
    };

    dir_path.rmdir(name.trim_end_matches('/'))?;
    Ok(SyscallReturn::Return(0))
}
