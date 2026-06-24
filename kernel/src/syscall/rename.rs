// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::{
        file::{InodeType, file_table::RawFileDesc},
        vfs::path::{AT_FDCWD, EmptyPathStr, FsPath, RenameMode, SplitPath},
    },
    prelude::*,
    syscall::constants::MAX_FILENAME_LEN,
};

pub fn sys_renameat2(
    old_dirfd: RawFileDesc,
    old_path_addr: Vaddr,
    new_dirfd: RawFileDesc,
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
    // TODO: Add support for handling the `WHITEOUT` flag.
    if flags.contains(Flags::WHITEOUT) || flags.contains(Flags::NOREPLACE | Flags::EXCHANGE) {
        return_errno_with_message!(Errno::EINVAL, "invalid renameat2 flag combination");
    }

    let fs_ref = ctx.thread_local.borrow_fs();
    let path_resolver = fs_ref.resolver().read();

    let old_path_name = old_path_name.to_string_lossy();
    let (old_parent_path, old_name) = {
        let (old_parent_path_name, old_name) = old_path_name.split_dirname_and_basename()?;
        let old_fs_path =
            FsPath::from_fd_at(old_dirfd, old_parent_path_name, EmptyPathStr::Reject)?;
        (path_resolver.lookup(&old_fs_path)?, old_name)
    };
    let old_path = path_resolver.lookup_at_path(&old_parent_path, old_name)?;
    if old_path.type_() != InodeType::Dir && old_path_name.ends_with('/') {
        return_errno_with_message!(Errno::ENOTDIR, "the old path is not a directory");
    }

    let new_path_name = new_path_name.to_string_lossy();
    let (new_parent_path, new_name) = {
        let (new_parent_path_name, new_name) = new_path_name.split_dirname_and_basename()?;
        let new_parent_fs_path =
            FsPath::from_fd_at(new_dirfd, new_parent_path_name, EmptyPathStr::Reject)?;
        (
            path_resolver.lookup(&new_parent_fs_path)?,
            new_name.to_string(),
        )
    };

    // Check for new path existence and type for exchange mode
    if flags.contains(Flags::EXCHANGE) {
        // Try to lookup new path
        let new_path_result = path_resolver.lookup_at_path(&new_parent_path, &new_name);
        let Ok(new_path) = new_path_result else {
            return_errno_with_message!(Errno::ENOENT, "the new path does not exist");
        };
        // Check type compatibility
        if (old_path.type_() == InodeType::Dir) != (new_path.type_() == InodeType::Dir) {
            if old_path.type_() == InodeType::Dir {
                return_errno_with_message!(Errno::ENOTDIR, "the new path is not a directory");
            } else {
                return_errno_with_message!(Errno::EISDIR, "the new path is a directory");
            }
        }
        // Check for circularity: old path cannot be an ancestor of new parent
        if old_path.type_() == InodeType::Dir
            && new_parent_path.is_equal_or_descendant_of(&old_path)
        {
            return_errno_with_message!(
                Errno::EINVAL,
                "the new path is inside the old directory or its subtree"
            );
        }
        // Check for circularity: new path cannot be an ancestor of old parent
        if new_path.type_() == InodeType::Dir
            && old_parent_path.is_equal_or_descendant_of(&new_path)
        {
            return_errno_with_message!(
                Errno::EINVAL,
                "the old path is inside the new directory or its subtree"
            );
        }
    } else {
        // Original checks for non-exchange modes
        if old_path.type_() != InodeType::Dir && new_path_name.ends_with('/') {
            return_errno_with_message!(Errno::EISDIR, "the new path is a directory");
        }
        if old_path.type_() == InodeType::Dir
            && new_parent_path.is_equal_or_descendant_of(&old_path)
        {
            return_errno_with_message!(
                Errno::EINVAL,
                "the new path is inside the old directory or its subtree"
            );
        }
    }

    let rename_mode = if flags.contains(Flags::EXCHANGE) {
        RenameMode::Exchange
    } else if flags.contains(Flags::NOREPLACE) {
        RenameMode::NoReplace
    } else {
        RenameMode::Replace
    };

    old_parent_path.rename(old_name, &new_parent_path, &new_name, rename_mode)?;

    Ok(SyscallReturn::Return(0))
}

pub fn sys_renameat(
    old_dirfd: RawFileDesc,
    old_path_addr: Vaddr,
    new_dirfd: RawFileDesc,
    new_path_addr: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    sys_renameat2(old_dirfd, old_path_addr, new_dirfd, new_path_addr, 0, ctx)
}

pub fn sys_rename(
    old_path_addr: Vaddr,
    new_path_addr: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    sys_renameat2(AT_FDCWD, old_path_addr, AT_FDCWD, new_path_addr, 0, ctx)
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
