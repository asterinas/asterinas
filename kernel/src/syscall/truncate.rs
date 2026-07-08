// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs,
    fs::{
        file::file_table::{RawFileDesc, get_file_fast},
        utils::PATH_MAX,
        vfs::path::{AT_FDCWD, EmptyPathStr, FsPath},
    },
    prelude::*,
    process::ResourceType,
    security::{self, FileSetattrKind},
};

pub fn sys_ftruncate(raw_fd: RawFileDesc, len: isize, ctx: &Context) -> Result<SyscallReturn> {
    debug!("raw_fd = {}, length = {}", raw_fd, len);

    check_length(len, ctx)?;

    let mut file_table = ctx.thread_local.borrow_file_table_mut();
    let file = get_file_fast!(&mut file_table, raw_fd.try_into()?);
    let path = file.path().clone();
    let fs_ref = ctx.thread_local.borrow_fs();
    let path_resolver = fs_ref.resolver().read();
    security::file_setattr(&path, &path_resolver, FileSetattrKind::Size)?;
    file.resize(len as usize)?;
    fs::vfs::notify::on_change(&path);
    Ok(SyscallReturn::Return(0))
}

pub fn sys_truncate(path_ptr: Vaddr, len: isize, ctx: &Context) -> Result<SyscallReturn> {
    let path_name = ctx.user_space().read_cstring(path_ptr, PATH_MAX)?;
    debug!("path = {:?}, length = {}", path_name, len);

    check_length(len, ctx)?;

    let path_name = path_name.to_string_lossy();
    let fs_path = FsPath::from_fd_at(AT_FDCWD, &path_name, EmptyPathStr::Reject)?;
    let fs_ref = ctx.thread_local.borrow_fs();
    let path_resolver = fs_ref.resolver().read();
    let dir_path = path_resolver.lookup(&fs_path)?;
    security::file_setattr(&dir_path, &path_resolver, FileSetattrKind::Size)?;
    dir_path.resize(len as usize)?;
    fs::vfs::notify::on_change(&dir_path);
    Ok(SyscallReturn::Return(0))
}

#[inline]
fn check_length(len: isize, ctx: &Context) -> Result<()> {
    if len < 0 {
        return_errno_with_message!(Errno::EINVAL, "length is negative");
    }

    let max_file_size = {
        let resource_limits = ctx.process.resource_limits();
        resource_limits
            .get_rlimit(ResourceType::RLIMIT_FSIZE)
            .get_cur() as usize
    };
    if len as usize > max_file_size {
        return_errno_with_message!(Errno::EFBIG, "length is larger than the maximum file size");
    }
    Ok(())
}
