// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::{
        file_table::{get_file_fast, FileDesc},
        fs_resolver::{FsPath, AT_FDCWD},
        utils::{InodeMode, PATH_MAX},
    },
    prelude::*,
};

pub fn sys_fchmod(fd: FileDesc, mode: u16, ctx: &Context) -> Result<SyscallReturn> {
    debug!("fd = {}, mode = 0o{:o}", fd, mode);

    let mut file_table = ctx.thread_local.borrow_file_table_mut();
    let file = get_file_fast!(&mut file_table, fd);
    file.set_mode(InodeMode::from_bits_truncate(mode))?;
    Ok(SyscallReturn::Return(0))
}

pub fn sys_chmod(path_ptr: Vaddr, mode: u16, ctx: &Context) -> Result<SyscallReturn> {
    self::sys_fchmodat(AT_FDCWD, path_ptr, mode, ctx)
}

// Glibc handles the `flags` argument, so we just ignore it.
pub fn sys_fchmodat(
    dirfd: FileDesc,
    path_ptr: Vaddr,
    mode: u16,
    /* flags: u32, */
    ctx: &Context,
) -> Result<SyscallReturn> {
    let path = ctx.user_space().read_cstring(path_ptr, PATH_MAX)?;
    debug!("dirfd = {}, path = {:?}, mode = 0o{:o}", dirfd, path, mode,);

    let dentry = {
        let path = path.to_string_lossy();
        if path.is_empty() {
            return_errno_with_message!(Errno::ENOENT, "path is empty");
        }
        let fs_path = FsPath::new(dirfd, path.as_ref())?;
        ctx.thread_local
            .borrow_fs()
            .resolver()
            .read()
            .lookup(&fs_path)?
    };
    dentry.set_mode(InodeMode::from_bits_truncate(mode))?;
    Ok(SyscallReturn::Return(0))
}
