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

pub fn sys_symlinkat(
    target_addr: Vaddr,
    dirfd: FileDesc,
    linkpath_addr: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let user_space = ctx.user_space();
    let target = user_space.read_cstring(target_addr, MAX_FILENAME_LEN)?;
    let linkpath = user_space.read_cstring(linkpath_addr, MAX_FILENAME_LEN)?;
    debug!(
        "target = {:?}, dirfd = {}, linkpath = {:?}",
        target, dirfd, linkpath
    );

    let target = target.to_string_lossy();
    if target.is_empty() {
        return_errno_with_message!(Errno::ENOENT, "target is empty");
    }
    let (dir_dentry, link_name) = {
        let linkpath = linkpath.to_string_lossy();
        if linkpath.is_empty() {
            return_errno_with_message!(Errno::ENOENT, "linkpath is empty");
        }
        let fs_path = FsPath::new(dirfd, linkpath.as_ref())?;
        ctx.posix_thread
            .fs()
            .resolver()
            .read()
            .lookup_dir_and_new_basename(&fs_path, false)?
    };

    let new_dentry = dir_dentry.new_fs_child(
        &link_name,
        InodeType::SymLink,
        InodeMode::from_bits_truncate(0o777),
    )?;
    new_dentry.inode().write_link(&target)?;
    Ok(SyscallReturn::Return(0))
}

pub fn sys_symlink(
    target_addr: Vaddr,
    linkpath_addr: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    self::sys_symlinkat(target_addr, AT_FDCWD, linkpath_addr, ctx)
}
