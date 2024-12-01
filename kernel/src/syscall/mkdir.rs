// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::{
        file_table::FileDesc,
        fs_resolver::{FsPath, AT_FDCWD},
        utils::{InodeMode, InodeType},
    },
    prelude::*,
    syscall::constants::MAX_FILENAME_LEN,
};

pub fn sys_mkdirat(
    dirfd: FileDesc,
    path_addr: Vaddr,
    mode: u16,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let path = ctx.user_space().read_cstring(path_addr, MAX_FILENAME_LEN)?;
    debug!("dirfd = {}, path = {:?}, mode = {}", dirfd, path, mode);

    let current = ctx.posix_thread;
    let (dir_dentry, name) = {
        let path = path.to_string_lossy();
        if path.is_empty() {
            return_errno_with_message!(Errno::ENOENT, "path is empty");
        }
        let fs_path = FsPath::new(dirfd, path.as_ref())?;
        current
            .fs()
            .resolver()
            .read()
            .lookup_dir_and_new_basename(&fs_path, true)?
    };

    let inode_mode = {
        let mask_mode = mode & !current.fs().umask().read().get();
        InodeMode::from_bits_truncate(mask_mode)
    };
    let _ = dir_dentry.new_fs_child(name.trim_end_matches('/'), InodeType::Dir, inode_mode)?;
    Ok(SyscallReturn::Return(0))
}

pub fn sys_mkdir(path_addr: Vaddr, mode: u16, ctx: &Context) -> Result<SyscallReturn> {
    self::sys_mkdirat(AT_FDCWD, path_addr, mode, ctx)
}
