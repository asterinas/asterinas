// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::{
        file_table::FileDesc,
        fs_resolver::{FsPath, AT_FDCWD},
        utils::PATH_MAX,
    },
    prelude::*,
    process::ResourceType,
};

pub fn sys_ftruncate(fd: FileDesc, len: isize, ctx: &Context) -> Result<SyscallReturn> {
    debug!("fd = {}, length = {}", fd, len);

    check_length(len, ctx)?;

    let file = {
        let file_table = ctx.posix_thread.file_table().lock();
        file_table.get_file(fd)?.clone()
    };

    file.resize(len as usize)?;
    Ok(SyscallReturn::Return(0))
}

pub fn sys_truncate(path_ptr: Vaddr, len: isize, ctx: &Context) -> Result<SyscallReturn> {
    let path = ctx.user_space().read_cstring(path_ptr, PATH_MAX)?;
    debug!("path = {:?}, length = {}", path, len);

    check_length(len, ctx)?;

    let dir_dentry = {
        let path = path.to_string_lossy();
        if path.is_empty() {
            return_errno_with_message!(Errno::ENOENT, "path is empty");
        }
        let fs_path = FsPath::new(AT_FDCWD, path.as_ref())?;
        ctx.posix_thread.fs().resolver().read().lookup(&fs_path)?
    };
    dir_dentry.resize(len as usize)?;
    Ok(SyscallReturn::Return(0))
}

#[inline]
fn check_length(len: isize, ctx: &Context) -> Result<()> {
    if len < 0 {
        return_errno_with_message!(Errno::EINVAL, "length is negative");
    }

    let max_file_size = {
        let resource_limits = ctx.process.resource_limits().lock();
        resource_limits
            .get_rlimit(ResourceType::RLIMIT_FSIZE)
            .get_cur() as usize
    };
    if len as usize > max_file_size {
        return_errno_with_message!(Errno::EFBIG, "length is larger than the maximum file size");
    }
    Ok(())
}
