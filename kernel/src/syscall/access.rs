// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::{
        file_table::FileDesc,
        fs_resolver::{FsPath, AT_FDCWD},
        utils::PATH_MAX,
    },
    prelude::*,
};

pub fn sys_faccessat(
    dirfd: FileDesc,
    path_ptr: Vaddr,
    mode: u16,
    ctx: &Context,
) -> Result<SyscallReturn> {
    debug!(
        "faccessat: dirfd = {}, path_ptr = {:#x}, mode = {:o}",
        dirfd, path_ptr, mode
    );

    do_faccessat(dirfd, path_ptr, mode, 0, ctx)
}

pub fn sys_access(path_ptr: Vaddr, mode: u16, ctx: &Context) -> Result<SyscallReturn> {
    debug!("access: path_ptr = {:#x}, mode = {:o}", path_ptr, mode);

    do_faccessat(AT_FDCWD, path_ptr, mode, 0, ctx)
}

bitflags! {
    struct FaccessatFlags: i32 {
        const AT_EACCESS = 0x200;
        const AT_SYMLINK_NOFOLLOW = 0x100;
        const AT_EMPTY_PATH = 0x1000;
    }
}

bitflags! {
    struct AccessMode: u16 {
        const R_OK = 0x4;
        const W_OK = 0x2;
        const X_OK = 0x1;
        // We could ignore F_OK in bitflags.
        // const F_OK = 0x0;
    }
}

pub fn do_faccessat(
    dirfd: FileDesc,
    path_ptr: Vaddr,
    mode: u16,
    flags: i32,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let mode = AccessMode::from_bits(mode)
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "Invalid mode"))?;
    let flags = FaccessatFlags::from_bits(flags)
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "Invalid flags"))?;

    let path = ctx.user_space().read_cstring(path_ptr, PATH_MAX)?;
    debug!(
        "dirfd = {}, path = {:?}, mode = {:o}, flags = {:?}",
        dirfd, path, mode, flags
    );

    let dentry = {
        let path = path.to_string_lossy();
        let fs_path = FsPath::new(dirfd, path.as_ref())?;
        let fs = ctx.posix_thread.fs().resolver().read();
        if flags.contains(FaccessatFlags::AT_SYMLINK_NOFOLLOW) {
            fs.lookup_no_follow(&fs_path)?
        } else {
            fs.lookup(&fs_path)?
        }
    };
    // AccessMode::empty() means F_OK and no more permission check needed.
    if mode.is_empty() {
        return Ok(SyscallReturn::Return(0));
    }

    let inode_mode = dentry.mode()?;

    // FIXME: The current implementation is dummy
    if mode.contains(AccessMode::R_OK) && !inode_mode.is_readable() {
        return_errno_with_message!(Errno::EACCES, "Read permission denied");
    }

    if mode.contains(AccessMode::W_OK) && !inode_mode.is_writable() {
        return_errno_with_message!(Errno::EACCES, "Write permission denied");
    }

    if mode.contains(AccessMode::X_OK) && !inode_mode.is_executable() {
        return_errno_with_message!(Errno::EACCES, "Execute permission denied");
    }

    Ok(SyscallReturn::Return(0))
}
