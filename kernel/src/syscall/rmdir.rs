// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::{
        file_table::FileDesc,
        fs_resolver::{FsPath, AT_FDCWD},
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
    let path = ctx.user_space().read_cstring(path_addr, MAX_FILENAME_LEN)?;
    debug!("dirfd = {}, path_addr = {:?}", dirfd, path_addr);

    let (dir_dentry, name) = {
        let path = path.to_string_lossy();
        if path.is_empty() {
            return_errno_with_message!(Errno::ENOENT, "path is empty");
        }
        if path.trim_end_matches('/').is_empty() {
            return_errno_with_message!(Errno::EBUSY, "is root directory");
        }
        let fs_path = FsPath::new(dirfd, path.as_ref())?;
        ctx.posix_thread
            .fs()
            .resolver()
            .read()
            .lookup_dir_and_base_name(&fs_path)?
    };
    dir_dentry.rmdir(name.trim_end_matches('/'))?;
    Ok(SyscallReturn::Return(0))
}
