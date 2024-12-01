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

pub fn sys_linkat(
    old_dirfd: FileDesc,
    old_path_addr: Vaddr,
    new_dirfd: FileDesc,
    new_path_addr: Vaddr,
    flags: u32,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let user_space = ctx.user_space();

    let old_path = user_space.read_cstring(old_path_addr, MAX_FILENAME_LEN)?;
    let new_path = user_space.read_cstring(new_path_addr, MAX_FILENAME_LEN)?;
    let flags =
        LinkFlags::from_bits(flags).ok_or(Error::with_message(Errno::EINVAL, "invalid flags"))?;
    debug!(
        "old_dirfd = {}, old_path = {:?}, new_dirfd = {}, new_path = {:?}, flags = {:?}",
        old_dirfd, old_path, new_dirfd, new_path, flags
    );

    let (old_dentry, new_dir_dentry, new_name) = {
        let old_path = old_path.to_string_lossy();
        if old_path.ends_with('/') {
            return_errno_with_message!(Errno::EPERM, "oldpath is dir");
        }
        if old_path.is_empty() && !flags.contains(LinkFlags::AT_EMPTY_PATH) {
            return_errno_with_message!(Errno::ENOENT, "oldpath is empty");
        }
        let new_path = new_path.to_string_lossy();
        if new_path.is_empty() {
            return_errno_with_message!(Errno::ENOENT, "newpath is empty");
        }

        let old_fs_path = FsPath::new(old_dirfd, old_path.as_ref())?;
        let new_fs_path = FsPath::new(new_dirfd, new_path.as_ref())?;
        let fs = ctx.posix_thread.fs().resolver().read();
        let old_dentry = if flags.contains(LinkFlags::AT_SYMLINK_FOLLOW) {
            fs.lookup(&old_fs_path)?
        } else {
            fs.lookup_no_follow(&old_fs_path)?
        };
        let (new_dir_dentry, new_name) = fs.lookup_dir_and_new_basename(&new_fs_path, false)?;
        (old_dentry, new_dir_dentry, new_name)
    };

    new_dir_dentry.link(&old_dentry, &new_name)?;
    Ok(SyscallReturn::Return(0))
}

pub fn sys_link(
    old_path_addr: Vaddr,
    new_path_addr: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    self::sys_linkat(AT_FDCWD, old_path_addr, AT_FDCWD, new_path_addr, 0, ctx)
}

bitflags::bitflags! {
    pub struct LinkFlags: u32 {
        const AT_EMPTY_PATH = 0x1000;
        const AT_SYMLINK_FOLLOW = 0x400;
    }
}
