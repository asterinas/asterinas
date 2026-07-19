// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::{
        file::{InodeType, file_table::RawFileDesc},
        vfs::path::{AT_FDCWD, EmptyPathStr, FsPath, SplitPath, SplitPathError},
    },
    prelude::*,
    syscall::constants::MAX_FILENAME_LEN,
};

pub fn sys_unlinkat(
    dirfd: RawFileDesc,
    path_addr: Vaddr,
    flags: u32,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let flags =
        UnlinkFlags::from_bits(flags).ok_or(Error::with_message(Errno::EINVAL, "invalid flags"))?;
    if flags.contains(UnlinkFlags::AT_REMOVEDIR) {
        return super::rmdir::sys_rmdirat(dirfd, path_addr, ctx);
    }

    let path_name = ctx.user_space().read_cstring(path_addr, MAX_FILENAME_LEN)?;
    debug!("dirfd = {}, path = {:?}", dirfd, path_name);

    let path_name = path_name.to_string_lossy();
    let (dir_path, name) = {
        let (parent_path_name, target_name) = path_name
            .split_dirname_and_basename()
            .map_err(SplitPathError::reject_root_as_is_dir)?;

        let fs_ref = ctx.thread_local.borrow_fs();
        let path_resolver = fs_ref.resolver().read();

        let parent_fs_path = FsPath::from_fd_at(dirfd, parent_path_name, EmptyPathStr::Reject)?;
        let dir_path = path_resolver.lookup(&parent_fs_path)?;

        // A trailing slash makes the unlink fail in one of two ways:
        // `dir/` with `EISDIR`, `file/` with `ENOTDIR`.
        if path_name.ends_with('/') {
            let target = path_resolver.lookup_at_path(&dir_path, target_name)?;
            if target.type_() == InodeType::Dir {
                return_errno_with_message!(Errno::EISDIR, "a directory cannot be unlinked");
            } else {
                return_errno_with_message!(
                    Errno::ENOTDIR,
                    "the path ends with a slash but is not a directory"
                );
            }
        }

        (dir_path, target_name)
    };

    dir_path.unlink(name)?;
    Ok(SyscallReturn::Return(0))
}

pub fn sys_unlink(path_addr: Vaddr, ctx: &Context) -> Result<SyscallReturn> {
    sys_unlinkat(AT_FDCWD, path_addr, 0, ctx)
}

bitflags::bitflags! {
    struct UnlinkFlags: u32 {
        const AT_REMOVEDIR = 0x200;
    }
}
