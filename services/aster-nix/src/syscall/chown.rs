// SPDX-License-Identifier: MPL-2.0

use super::{SyscallReturn, SYS_FCHOWN, SYS_FCHOWNAT};
use crate::{
    fs::{
        file_table::FileDescripter,
        fs_resolver::{FsPath, AT_FDCWD},
        utils::PATH_MAX,
    },
    log_syscall_entry,
    prelude::*,
    process::{Gid, Uid},
    util::read_cstring_from_user,
};

pub fn sys_fchown(fd: FileDescripter, uid: i32, gid: i32) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_FCHOWN);
    debug!("fd = {}, uid = {}, gid = {}", fd, uid, gid);

    let uid = to_optional_id(uid, Uid::new)?;
    let gid = to_optional_id(gid, Gid::new)?;
    if uid.is_none() && gid.is_none() {
        return Ok(SyscallReturn::Return(0));
    }

    let current = current!();
    let file_table = current.file_table().lock();
    let file = file_table.get_file(fd)?;
    if let Some(uid) = uid {
        file.set_owner(uid)?;
    }
    if let Some(gid) = gid {
        file.set_group(gid)?;
    }
    Ok(SyscallReturn::Return(0))
}

pub fn sys_chown(path_ptr: Vaddr, uid: i32, gid: i32) -> Result<SyscallReturn> {
    self::sys_fchownat(AT_FDCWD, path_ptr, uid, gid, 0)
}

pub fn sys_lchown(path_ptr: Vaddr, uid: i32, gid: i32) -> Result<SyscallReturn> {
    self::sys_fchownat(
        AT_FDCWD,
        path_ptr,
        uid,
        gid,
        ChownFlags::AT_SYMLINK_NOFOLLOW.bits(),
    )
}

pub fn sys_fchownat(
    dirfd: FileDescripter,
    path_ptr: Vaddr,
    uid: i32,
    gid: i32,
    flags: u32,
) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_FCHOWNAT);
    let path = read_cstring_from_user(path_ptr, PATH_MAX)?;
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
        return self::sys_fchown(dirfd, uid, gid);
    }

    let uid = to_optional_id(uid, Uid::new)?;
    let gid = to_optional_id(gid, Gid::new)?;
    if uid.is_none() && gid.is_none() {
        return Ok(SyscallReturn::Return(0));
    }

    let current = current!();
    let dentry = {
        let path = path.to_string_lossy();
        let fs_path = FsPath::new(dirfd, path.as_ref())?;
        let fs = current.fs().read();
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
