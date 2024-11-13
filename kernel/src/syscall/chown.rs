// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::{
        file_table::FileDesc,
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

    let file = {
        let file_table = ctx.process.file_table().lock();
        file_table.get_file(fd)?.clone()
    };
    if let Some(uid) = uid {
        file.set_owner(uid)?;
    }
    if let Some(gid) = gid {
        file.set_group(gid)?;
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
    let path = ctx.user_space().read_cstring(path_ptr, PATH_MAX)?;
    let flags = ChownFlags::from_bits(flags)
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "invalid flags"))?;
    debug!(
        "dirfd = {}, path = {:?}, uid = {}, gid = {}, flags = {:?}",
        dirfd, path, uid, gid, flags
    );

    if path.is_empty() {
        if !flags.contains(ChownFlags::AT_EMPTY_PATH) {
            return_errno_with_message!(Errno::ENOENT, "path is empty");
        }
        return self::sys_fchown(dirfd, uid, gid, ctx);
    }

    let uid = to_optional_id(uid, Uid::new)?;
    let gid = to_optional_id(gid, Gid::new)?;
    if uid.is_none() && gid.is_none() {
        return Ok(SyscallReturn::Return(0));
    }

    let dentry = {
        let path = path.to_string_lossy();
        let fs_path = FsPath::new(dirfd, path.as_ref())?;
        let fs = ctx.process.fs().read();
        if flags.contains(ChownFlags::AT_SYMLINK_NOFOLLOW) {
            fs.lookup_no_follow(&fs_path)?
        } else {
            fs.lookup(&fs_path)?
        }
    };
    if let Some(uid) = uid {
        dentry.set_owner(uid)?;
    }
    if let Some(gid) = gid {
        dentry.set_group(gid)?;
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
