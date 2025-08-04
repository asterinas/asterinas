// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::{
        file_table::FileDesc,
        fs_resolver::{FsPath, AT_FDCWD},
        utils::InodeType,
    },
    prelude::*,
    syscall::constants::MAX_FILENAME_LEN,
};

pub fn sys_renameat(
    old_dirfd: FileDesc,
    old_path_addr: Vaddr,
    new_dirfd: FileDesc,
    new_path_addr: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let user_space = ctx.user_space();
    let old_path_name = user_space.read_cstring(old_path_addr, MAX_FILENAME_LEN)?;
    let new_path_name = user_space.read_cstring(new_path_addr, MAX_FILENAME_LEN)?;
    debug!(
        "old_dirfd = {}, old_path = {:?}, new_dirfd = {}, new_path = {:?}",
        old_dirfd, old_path_name, new_dirfd, new_path_name
    );

    let fs_ref = ctx.thread_local.borrow_fs();
    let fs = fs_ref.resolver().read();

    let (old_dir_path, old_name) = {
        let old_path_name = old_path_name.to_string_lossy();
        if old_path_name.is_empty() {
            return_errno_with_message!(Errno::ENOENT, "oldpath is empty");
        }
        let old_fs_path = FsPath::new(old_dirfd, old_path_name.as_ref())?;
        fs.lookup_dir_and_base_name(&old_fs_path)?
    };
    let old_path = old_dir_path.lookup(&old_name)?;

    let (new_dir_path, new_name) = {
        let new_path_name = new_path_name.to_string_lossy();
        if new_path_name.is_empty() {
            return_errno_with_message!(Errno::ENOENT, "newpath is empty");
        }
        if new_path_name.ends_with('/') && old_path.type_() != InodeType::Dir {
            return_errno_with_message!(Errno::ENOTDIR, "oldpath is not dir");
        }
        let new_fs_path = FsPath::new(new_dirfd, new_path_name.as_ref().trim_end_matches('/'))?;
        fs.lookup_dir_and_base_name(&new_fs_path)?
    };

    // Check abs_path
    let old_abs_path = old_path.abs_path();
    let new_abs_path = new_dir_path.abs_path() + "/" + &new_name;
    if new_abs_path.starts_with(&old_abs_path) {
        if new_abs_path.len() == old_abs_path.len() {
            return Ok(SyscallReturn::Return(0));
        } else {
            return_errno_with_message!(
                Errno::EINVAL,
                "newpath contains a path prefix of the oldpath"
            );
        }
    }

    old_dir_path.rename(&old_name, &new_dir_path, &new_name)?;

    Ok(SyscallReturn::Return(0))
}

pub fn sys_rename(
    old_path_addr: Vaddr,
    new_path_addr: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    self::sys_renameat(AT_FDCWD, old_path_addr, AT_FDCWD, new_path_addr, ctx)
}
