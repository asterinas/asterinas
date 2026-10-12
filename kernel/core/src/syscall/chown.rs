// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs,
    fs::{
        file::{
            InodeMode, InodeType,
            file_table::{RawFileDesc, get_file_fast},
        },
        utils::PATH_MAX,
        vfs::path::{AT_FDCWD, EmptyPathStr, FsPath, Path},
    },
    prelude::*,
    process::{Gid, Uid, credentials::capabilities::CapSet},
    security::lsm::hooks as lsm_hooks,
};

pub(super) fn sys_fchown(
    raw_fd: RawFileDesc,
    uid: i32,
    gid: i32,
    ctx: &Context,
) -> Result<SyscallReturn> {
    debug!("raw_fd = {}, uid = {}, gid = {}", raw_fd, uid, gid);

    let uid = to_optional_id(uid, Uid::new)?;
    let gid = to_optional_id(gid, Gid::new)?;

    let mut file_table = ctx.thread_local.borrow_file_table_mut();
    let file = get_file_fast!(&mut file_table, raw_fd.try_into()?);
    do_chown(file.path(), uid, gid, ctx)?;
    Ok(SyscallReturn::Return(0))
}

pub(super) fn sys_chown(
    path_ptr: Vaddr,
    uid: i32,
    gid: i32,
    ctx: &Context,
) -> Result<SyscallReturn> {
    sys_fchownat(AT_FDCWD, path_ptr, uid, gid, 0, ctx)
}

pub(super) fn sys_lchown(
    path_ptr: Vaddr,
    uid: i32,
    gid: i32,
    ctx: &Context,
) -> Result<SyscallReturn> {
    sys_fchownat(
        AT_FDCWD,
        path_ptr,
        uid,
        gid,
        ChownFlags::AT_SYMLINK_NOFOLLOW.bits(),
        ctx,
    )
}

pub(super) fn sys_fchownat(
    dirfd: RawFileDesc,
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

    let uid = to_optional_id(uid, Uid::new)?;
    let gid = to_optional_id(gid, Gid::new)?;

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

    do_chown(&path, uid, gid, ctx)?;
    Ok(SyscallReturn::Return(0))
}

/// Changes the owner and/or the group of the file at `path`.
///
/// The permission checks mirror Linux's `chown_ok()` and `chgrp_ok()`:
///
///  * changing the owner requires `CAP_CHOWN`;
///  * changing the group requires `CAP_CHOWN` as well, except that the owner
///    of the file may change the group to one it is a member of.
///
/// A successful `chown` on a non-directory also strips the set-user-ID and
/// set-group-ID bits like Linux does: the set-user-ID bit is always cleared,
/// while the set-group-ID bit is preserved only when the file is not
/// group-executable and the caller is a member of the file's group (or has
/// `CAP_FSETID`).  Like every other mode change, the stripping requires the
/// owner or `CAP_FOWNER`, and it happens even when neither ID is changed.
fn do_chown(path: &Path, uid: Option<Uid>, gid: Option<Gid>, ctx: &Context) -> Result<()> {
    let metadata = path.metadata()?;
    let credentials = ctx.posix_thread.credentials();
    let is_owner = credentials.fsuid() == metadata.uid;

    // A no-op by the file's owner passes; changing the owner needs `CAP_CHOWN`.
    let may_set_uid = uid.is_none_or(|uid| is_owner && uid == metadata.uid);

    // The owner may keep the group or move it into one of its groups; every
    // other change needs `CAP_CHOWN`.
    let may_set_gid = gid.is_none_or(|gid| {
        let is_in_new_group = credentials.fsgid() == gid || credentials.groups().contains(&gid);
        is_owner && (gid == metadata.gid || is_in_new_group)
    });

    // Compute the mode that a successful chown would leave behind: Linux strips
    // the set-user-ID bit of a non-directory, and the set-group-ID bit unless
    // the file is not group-executable and the caller is in the file's group
    // (or has `CAP_FSETID`).
    let mut mode = metadata.mode;
    if !matches!(path.type_(), InodeType::Dir) {
        mode.remove(InodeMode::S_ISUID);
        if mode.has_set_gid() {
            let is_in_file_group =
                credentials.fsgid() == metadata.gid || credentials.groups().contains(&metadata.gid);
            let keeps_set_gid = !mode.is_group_executable()
                && (is_in_file_group || require_cap(ctx, CapSet::FSETID).is_ok());
            if !keeps_set_gid {
                mode.remove(InodeMode::S_ISGID);
            }
        }
    }

    // Check the whole request before writing anything, so that a rejected call
    // leaves the file untouched.
    let no_cap_needed = may_set_uid && may_set_gid;
    if !no_cap_needed {
        require_cap(ctx, CapSet::CHOWN)?;
    }
    if mode != metadata.mode {
        // Linux applies the stripping as a mode change, which
        // `setattr_prepare()` allows only for the owner or with `CAP_FOWNER`.
        let may_change_mode = is_owner || require_cap(ctx, CapSet::FOWNER).is_ok();
        if !may_change_mode {
            return_errno_with_message!(Errno::EPERM, "cannot change the file mode");
        }
    }

    if let Some(uid) = uid {
        path.set_owner(uid)?;
    }
    if let Some(gid) = gid {
        path.set_group(gid)?;
    }
    if mode != metadata.mode {
        path.set_mode(mode)?;
    }

    fs::vfs::notify::on_attr_change(path);

    Ok(())
}

fn require_cap(ctx: &Context, cap: CapSet) -> Result<()> {
    lsm_hooks::on_capable(lsm_hooks::CapableContext::new(
        ctx.thread_local.borrow_user_ns().as_ref(),
        ctx.posix_thread,
        cap,
    ))
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
