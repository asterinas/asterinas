// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs,
    fs::{
        file_table::FileDesc,
        fs_resolver::{AT_FDCWD, FsPath},
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

    let fs_ref = ctx.thread_local.borrow_fs();
    let (dir_path, name) = {
        let path = path.to_string_lossy();
        let fs_path = FsPath::from_fd_and_path(dirfd, &path)?;
        fs_ref
            .resolver()
            .read()
            .lookup_unresolved_no_follow(&fs_path)?
            .into_parent_and_basename()?
    };

    let inode_mode = {
        let mask_mode = mode & !fs_ref.umask().get();
        InodeMode::from_bits_truncate(mask_mode)
    };
    dir_path.new_fs_child(&name, InodeType::Dir, inode_mode)?;
    fs::notify::on_mkdir(&dir_path, || name);
    Ok(SyscallReturn::Return(0))
}

pub fn sys_mkdir(path_addr: Vaddr, mode: u16, ctx: &Context) -> Result<SyscallReturn> {
    self::sys_mkdirat(AT_FDCWD, path_addr, mode, ctx)
}
