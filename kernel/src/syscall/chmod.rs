// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs,
    fs::{
        file_table::{get_file_fast, FileDesc},
        fs_resolver::{FsPath, AT_FDCWD},
        utils::{InodeMode, PATH_MAX},
    },
    prelude::*,
};

pub fn sys_fchmod(fd: FileDesc, mode: u16, ctx: &Context) -> Result<SyscallReturn> {
    debug!("fd = {}, mode = 0o{:o}", fd, mode);

    let mut file_table = ctx.thread_local.borrow_file_table_mut();
    let file = get_file_fast!(&mut file_table, fd);
    file.inode().set_mode(InodeMode::from_bits_truncate(mode))?;
    if let Some(path) = file.path() {
        fs::notify::on_attr_change(path);
    }
    Ok(SyscallReturn::Return(0))
}

pub fn sys_chmod(path_ptr: Vaddr, mode: u16, ctx: &Context) -> Result<SyscallReturn> {
    do_fchmodat(AT_FDCWD, path_ptr, mode, ChmodFlags::empty(), ctx)
}

pub fn sys_fchmodat(
    dirfd: FileDesc,
    path_ptr: Vaddr,
    mode: u16,
    ctx: &Context,
) -> Result<SyscallReturn> {
    do_fchmodat(dirfd, path_ptr, mode, ChmodFlags::empty(), ctx)
}

pub fn sys_fchmodat2(
    dirfd: FileDesc,
    path_ptr: Vaddr,
    mode: u16,
    flags: u32,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let flags = ChmodFlags::from_bits(flags)
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "invalid chmod flags"))?;

    do_fchmodat(dirfd, path_ptr, mode, flags, ctx)
}

fn do_fchmodat(
    dirfd: FileDesc,
    path_ptr: Vaddr,
    mode: u16,
    flags: ChmodFlags,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let path_name = ctx.user_space().read_cstring(path_ptr, PATH_MAX)?;

    debug!(
        "dirfd = {}, path_name = {:?}, mode = 0o{:o}, flags = {:?}",
        dirfd, path_name, mode, flags,
    );

    let path_or_inode = {
        let path_name = path_name.to_string_lossy();
        let fs_path = if flags.contains(ChmodFlags::AT_EMPTY_PATH) && path_name.is_empty() {
            FsPath::from_fd(dirfd)?
        } else {
            FsPath::from_fd_and_path(dirfd, &path_name)?
        };

        let fs_ref = ctx.thread_local.borrow_fs();
        let fs = fs_ref.resolver().read();
        if flags.contains(ChmodFlags::AT_SYMLINK_NOFOLLOW) {
            fs.lookup_inode_no_follow(&fs_path)?
        } else {
            fs.lookup_inode(&fs_path)?
        }
    };

    path_or_inode
        .inode()
        .set_mode(InodeMode::from_bits_truncate(mode))?;
    if let Some(path) = path_or_inode.into_path() {
        fs::notify::on_attr_change(&path);
    }
    Ok(SyscallReturn::Return(0))
}

bitflags::bitflags! {
    struct ChmodFlags: u32 {
        const AT_EMPTY_PATH = 1 << 12;
        const AT_SYMLINK_NOFOLLOW = 1 << 8;
    }
}
