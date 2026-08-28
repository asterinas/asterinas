// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::{
        file::{Permission, file_table::RawFileDesc},
        utils::PATH_MAX,
        vfs::path::{AT_FDCWD, EmptyPathStr, FsPath},
    },
    prelude::*,
};

pub(super) fn sys_faccessat(
    dirfd: RawFileDesc,
    path_ptr: Vaddr,
    mode: u32,
    ctx: &Context,
) -> Result<SyscallReturn> {
    debug!(
        "faccessat: dirfd = {}, path_ptr = {:#x}, mode = {:o}",
        dirfd, path_ptr, mode
    );

    do_faccessat(dirfd, path_ptr, mode, 0, ctx)
}

pub(super) fn sys_access(path_ptr: Vaddr, mode: u32, ctx: &Context) -> Result<SyscallReturn> {
    debug!("access: path_ptr = {:#x}, mode = {:o}", path_ptr, mode);

    do_faccessat(AT_FDCWD, path_ptr, mode, 0, ctx)
}

pub(super) fn sys_faccessat2(
    dirfd: RawFileDesc,
    path_ptr: Vaddr,
    mode: u32,
    flags: u32,
    ctx: &Context,
) -> Result<SyscallReturn> {
    debug!(
        "faccessat2: dirfd = {}, path_ptr = {:#x}, mode = {:o}, flags = {}",
        dirfd, path_ptr, mode, flags
    );

    do_faccessat(dirfd, path_ptr, mode, flags, ctx)
}

bitflags! {
    struct FaccessatFlags: u32 {
        const AT_EACCESS = 0x200;
        const AT_SYMLINK_NOFOLLOW = 0x100;
        const AT_EMPTY_PATH = 0x1000;
    }
}

bitflags! {
    /// The accessibility checks requested by the `access(2)` family.
    ///
    /// Do not confuse this type with [`crate::fs::file::AccessMode`],
    /// which describes the read/write mode of an opened file.
    struct AccessMode: u32 {
        const R_OK = 0x4;
        const W_OK = 0x2;
        const X_OK = 0x1;
        // We should ignore `F_OK` in bitflags.
        // const F_OK = 0x0;
    }
}

impl AccessMode {
    /// Returns the permissions that the access mode requires.
    ///
    /// `F_OK` is represented by `AccessMode::empty()`,
    /// which requires no permissions.
    fn required_permission(&self) -> Permission {
        let mut permission = Permission::empty();

        if self.contains(Self::R_OK) {
            permission |= Permission::MAY_READ;
        }
        if self.contains(Self::W_OK) {
            permission |= Permission::MAY_WRITE;
        }
        if self.contains(Self::X_OK) {
            permission |= Permission::MAY_EXEC;
        }

        permission
    }
}

fn do_faccessat(
    dirfd: RawFileDesc,
    path_ptr: Vaddr,
    mode: u32,
    flags: u32,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let mode = AccessMode::from_bits(mode)
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "invalid mode"))?;
    let flags = FaccessatFlags::from_bits(flags)
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "invalid flags"))?;

    let path_name = ctx.user_space().read_cstring(path_ptr, PATH_MAX)?;
    debug!(
        "dirfd = {}, path_name = {:?}, mode = {:o}, flags = {:?}",
        dirfd, path_name, mode, flags
    );

    let path = {
        let path_name = path_name.to_string_lossy();
        let fs_path =
            FsPath::from_fd_at(dirfd, &path_name, EmptyPathStr::AllowIfFlag(flags.bits()))?;

        let fs_ref = ctx.thread_local.borrow_fs();
        let path_resolver = fs_ref.resolver().read();
        if flags.contains(FaccessatFlags::AT_SYMLINK_NOFOLLOW) {
            path_resolver.lookup_no_follow(&fs_path)?
        } else {
            path_resolver.lookup(&fs_path)?
        }
    };

    let inode = path.inode();

    // `F_OK` is represented by `AccessMode::empty()`.
    // It only asks whether the file exists,
    // which the successful path lookup above has already established.
    // So we can skip the permission check, which is not free.
    let permission = mode.required_permission();
    if !permission.is_empty() {
        inode.check_permission(permission)?;
    }

    Ok(SyscallReturn::Return(0))
}
