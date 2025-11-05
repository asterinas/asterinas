// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::{
        file_table::{get_file_fast, FileDesc},
        fs_resolver::{FsPath, AT_FDCWD},
        utils::PATH_MAX,
    },
    prelude::*,
    process::{Gid, Uid},
};

pub fn sys_fchown(fd: FileDesc, uid: i32, gid: i32, ctx: &Context) -> Result<SyscallReturn> {
    debug!("fd = {}, uid = {}, gid = {}", fd, uid, gid);

    let uid = to_optional_id(uid, Uid::new)?;
    let gid = to_optional_id(gid, Gid::new)?;
    if uid.is_none() && gid.is_none() {
        return Ok(SyscallReturn::Return(0));
    }

    let mut file_table = ctx.thread_local.borrow_file_table_mut();
    let file = get_file_fast!(&mut file_table, fd);
    if let Some(uid) = uid {
        file.inode().set_owner(uid)?;
    }
    if let Some(gid) = gid {
        file.inode().set_group(gid)?;
    }
    Ok(SyscallReturn::Return(0))
}

pub fn sys_chown(path_ptr: Vaddr, uid: i32, gid: i32, ctx: &Context) -> Result<SyscallReturn> {
    self::sys_fchownat(AT_FDCWD, path_ptr, uid, gid, 0, ctx)
}

pub fn sys_lchown(path_ptr: Vaddr, uid: i32, gid: i32, ctx: &Context) -> Result<SyscallReturn> {
    self::sys_fchownat(
        AT_FDCWD,
        path_ptr,
        uid,
        gid,
        ChownFlags::AT_SYMLINK_NOFOLLOW.bits(),
        ctx,
    )
}

pub fn sys_fchownat(
    dirfd: FileDesc,
    path_ptr: Vaddr,
    uid: i32,
    gid: i32,
    flags: u32,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let path_name = ctx.user_space().read_cstring(path_ptr, PATH_MAX)?;
    let flags = ChownFlags::from_bits(flags)
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "invalid flags"))?;
    debug!(
        "dirfd = {}, path = {:?}, uid = {}, gid = {}, flags = {:?}",
        dirfd, path_name, uid, gid, flags
    );

    if flags.contains(ChownFlags::AT_EMPTY_PATH) && path_name.is_empty() {
        return self::sys_fchown(dirfd, uid, gid, ctx);
    }

    let uid = to_optional_id(uid, Uid::new)?;
    let gid = to_optional_id(gid, Gid::new)?;
    if uid.is_none() && gid.is_none() {
        return Ok(SyscallReturn::Return(0));
    }

    let path_or_inode = {
        let path_name = path_name.to_string_lossy();
        let fs_path = FsPath::from_fd_and_path(dirfd, &path_name)?;

        let fs_ref = ctx.thread_local.borrow_fs();
        let fs = fs_ref.resolver().read();
        if flags.contains(ChownFlags::AT_SYMLINK_NOFOLLOW) {
            fs.lookup_inode_no_follow(&fs_path)?
        } else {
            fs.lookup_inode(&fs_path)?
        }
    };

    let inode = path_or_inode.inode();

    if let Some(uid) = uid {
        inode.set_owner(uid)?;
    }
    if let Some(gid) = gid {
        inode.set_group(gid)?;
    }
    Ok(SyscallReturn::Return(0))
}

fn to_optional_id<T>(id: i32, f: impl Fn(u32) -> T) -> Result<Option<T>> {
    let id = if id >= 0 {
        Some(f(id as u32))
    } else if id == -1 {
        // If the owner or group is specified as -1, then that ID is not changed.
        None
    } else {
        return_errno!(Errno::EINVAL);
    };

    Ok(id)
}

bitflags! {
    struct ChownFlags: u32 {
        const AT_SYMLINK_NOFOLLOW = 1 << 8;
        const AT_EMPTY_PATH = 1 << 12;
    }
}
