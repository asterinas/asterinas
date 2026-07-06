// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::{
        file::file_table::{RawFileDesc, get_file_fast},
        utils::PATH_MAX,
        vfs::path::{AT_FDCWD, EmptyPathStr, FsPath},
    },
    prelude::*,
    process::{Gid, RawGid, RawUid, Uid},
};

pub fn sys_fchown(
    raw_fd: RawFileDesc,
    raw_uid: RawUid,
    raw_gid: RawGid,
    ctx: &Context,
) -> Result<SyscallReturn> {
    debug!(
        "raw_fd = {}, raw_uid = {}, raw_gid = {}",
        raw_fd, raw_uid, raw_gid
    );

    let uid = Uid::new(raw_uid);
    let gid = Gid::new(raw_gid);
    if uid.is_none() && gid.is_none() {
        return Ok(SyscallReturn::Return(0));
    }

    let mut file_table = ctx.thread_local.borrow_file_table_mut();
    let file = get_file_fast!(&mut file_table, raw_fd.try_into()?);
    let path = file.path();
    if let Some(uid) = uid {
        path.set_owner(uid)?;
    }
    if let Some(gid) = gid {
        path.set_group(gid)?;
    }
    Ok(SyscallReturn::Return(0))
}

pub fn sys_chown(
    path_ptr: Vaddr,
    raw_uid: RawUid,
    raw_gid: RawGid,
    ctx: &Context,
) -> Result<SyscallReturn> {
    sys_fchownat(AT_FDCWD, path_ptr, raw_uid, raw_gid, 0, ctx)
}

pub fn sys_lchown(
    path_ptr: Vaddr,
    raw_uid: RawUid,
    raw_gid: RawGid,
    ctx: &Context,
) -> Result<SyscallReturn> {
    sys_fchownat(
        AT_FDCWD,
        path_ptr,
        raw_uid,
        raw_gid,
        ChownFlags::AT_SYMLINK_NOFOLLOW.bits(),
        ctx,
    )
}

pub fn sys_fchownat(
    dirfd: RawFileDesc,
    path_ptr: Vaddr,
    raw_uid: RawUid,
    raw_gid: RawGid,
    flags: u32,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let path_name = ctx.user_space().read_cstring(path_ptr, PATH_MAX)?;
    let flags = ChownFlags::from_bits(flags)
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "invalid flags"))?;
    debug!(
        "dirfd = {}, path = {:?}, raw_uid = {}, raw_gid = {}, flags = {:?}",
        dirfd, path_name, raw_uid, raw_gid, flags
    );

    if flags.contains(ChownFlags::AT_EMPTY_PATH) && path_name.is_empty() {
        return sys_fchown(dirfd, raw_uid, raw_gid, ctx);
    }

    let uid = Uid::new(raw_uid);
    let gid = Gid::new(raw_gid);
    if uid.is_none() && gid.is_none() {
        return Ok(SyscallReturn::Return(0));
    }

    let path = {
        let path_name = path_name.to_string_lossy();
        let fs_path =
            FsPath::from_fd_at(dirfd, &path_name, EmptyPathStr::AllowIfFlag(flags.bits()))?;

        let fs_ref = ctx.thread_local.borrow_fs();
        let path_resolver = fs_ref.resolver().read();
        if flags.contains(ChownFlags::AT_SYMLINK_NOFOLLOW) {
            path_resolver.lookup_no_follow(&fs_path)?
        } else {
            path_resolver.lookup(&fs_path)?
        }
    };

    if let Some(uid) = uid {
        path.set_owner(uid)?;
    }
    if let Some(gid) = gid {
        path.set_group(gid)?;
    }
    Ok(SyscallReturn::Return(0))
}

bitflags! {
    struct ChownFlags: u32 {
        const AT_SYMLINK_NOFOLLOW = 1 << 8;
        const AT_EMPTY_PATH = 1 << 12;
    }
}
