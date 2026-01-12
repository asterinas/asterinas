// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::{
        file_table::FileDesc,
        path::{AT_FDCWD, FsPath},
        utils::InodeType,
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

    let old_path_name = user_space.read_cstring(old_path_addr, MAX_FILENAME_LEN)?;
    let new_path_name = user_space.read_cstring(new_path_addr, MAX_FILENAME_LEN)?;
    let flags: LinkFlags =
        LinkFlags::from_bits(flags).ok_or(Error::with_message(Errno::EINVAL, "invalid flags"))?;
    debug!(
        "old_dirfd = {}, old_path = {:?}, new_dirfd = {}, new_path = {:?}, flags = {:?}",
        old_dirfd, old_path_name, new_dirfd, new_path_name, flags
    );

    let (old_path, new_path, new_name) = {
        let old_path_name = old_path_name.to_string_lossy();
        let new_path_name = new_path_name.to_string_lossy();

        let old_fs_path = if flags.contains(LinkFlags::AT_EMPTY_PATH) && old_path_name.is_empty() {
            FsPath::from_fd(old_dirfd)?
        } else {
            FsPath::from_fd_and_path(old_dirfd, &old_path_name)?
        };
        let new_fs_path = FsPath::from_fd_and_path(new_dirfd, &new_path_name)?;

        let fs_ref = ctx.thread_local.borrow_fs();
        let path_resolver = fs_ref.resolver().read();

        let old_path = if flags.contains(LinkFlags::AT_SYMLINK_FOLLOW) {
            path_resolver.lookup(&old_fs_path)?
        } else {
            path_resolver.lookup_no_follow(&old_fs_path)?
        };
        if old_path.type_() == InodeType::Dir {
            return_errno_with_message!(Errno::EPERM, "the link path is a directory");
        }

        let (new_path, new_name) = path_resolver
            .lookup_unresolved_no_follow(&new_fs_path)?
            .into_parent_and_filename()?;

        (old_path, new_path, new_name)
    };

    new_path.link(&old_path, &new_name)?;
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
