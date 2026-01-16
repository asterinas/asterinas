// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::{
        file_table::FileDesc,
        path::{AT_FDCWD, FsPath, SplitPath},
        utils::InodeType,
    },
    prelude::*,
    syscall::constants::MAX_FILENAME_LEN,
};

pub fn sys_renameat2(
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
    debug!(
        "old_dirfd = {}, old_path = {:?}, new_dirfd = {}, new_path = {:?}",
        old_dirfd, old_path_name, new_dirfd, new_path_name
    );
    let Some(flags) = Flags::from_bits(flags) else {
        return_errno_with_message!(Errno::EINVAL, "invalid flags");
    };
    // TODO: Add support for handling the `NOREPLACE`, `EXCHANGE`, and `WHITEOUT` flags.
    if !flags.is_empty() {
        warn!("unsupported flags: {:?}", flags);
        return_errno_with_message!(Errno::EINVAL, "unsupported flags");
    }

    let fs_ref = ctx.thread_local.borrow_fs();
    let path_resolver = fs_ref.resolver().read();

    let old_path_name = old_path_name.to_string_lossy();
    let (old_dir_path, old_name) = {
        let (old_parent_path_name, old_name) = old_path_name.split_dirname_and_basename()?;
        let old_fs_path = FsPath::from_fd_and_path(old_dirfd, old_parent_path_name)?;
        (path_resolver.lookup(&old_fs_path)?, old_name)
    };
    let old_path = path_resolver.lookup_at_path(&old_dir_path, old_name)?;
    if old_path.type_() != InodeType::Dir && old_path_name.ends_with('/') {
        return_errno_with_message!(Errno::ENOTDIR, "the old path is not a directory");
    }

    let new_path_name = new_path_name.to_string_lossy();
    let (new_dir_path, new_name) = {
        if old_path.type_() != InodeType::Dir && new_path_name.ends_with('/') {
            return_errno_with_message!(Errno::EISDIR, "the new path is a directory");
        }
        let (new_parent_path_name, new_name) = new_path_name.split_dirname_and_basename()?;
        let new_fs_path = FsPath::from_fd_and_path(new_dirfd, new_parent_path_name)?;
        (path_resolver.lookup(&new_fs_path)?, new_name)
    };

    // Check the absolute path
    // FIXME: Using string prefix matching to check for path containment is incorrect.
    // It doesn't handle path components like '..' or '.' properly.
    let old_abs_path = path_resolver.make_abs_path(&old_path).into_string();
    let new_abs_path = path_resolver.make_abs_path(&new_dir_path).into_string() + "/" + new_name;
    if new_abs_path.starts_with(&old_abs_path) {
        if new_abs_path.len() == old_abs_path.len() {
            return Ok(SyscallReturn::Return(0));
        } else {
            return_errno_with_message!(
                Errno::EINVAL,
                "the new path contains a path prefix of the old path"
            );
        }
    }

    old_dir_path.rename(old_name, &new_dir_path, new_name)?;

    Ok(SyscallReturn::Return(0))
}

pub fn sys_renameat(
    old_dirfd: FileDesc,
    old_path_addr: Vaddr,
    new_dirfd: FileDesc,
    new_path_addr: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    self::sys_renameat2(old_dirfd, old_path_addr, new_dirfd, new_path_addr, 0, ctx)
}

pub fn sys_rename(
    old_path_addr: Vaddr,
    new_path_addr: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    self::sys_renameat2(AT_FDCWD, old_path_addr, AT_FDCWD, new_path_addr, 0, ctx)
}

bitflags! {
    /// Flags used in the `renameat2` system call.
    ///
    /// Reference: <https://elixir.bootlin.com/linux/v6.16.3/source/include/uapi/linux/fcntl.h#L140-L143>.
    struct Flags: u32 {
        const NOREPLACE = 1 << 0;
        const EXCHANGE  = 1 << 1;
        const WHITEOUT  = 1 << 2;
    }
}
