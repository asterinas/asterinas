// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::fs_resolver::{FsPath, AT_FDCWD},
    prelude::*,
    syscall::constants::MAX_FILENAME_LEN,
};

pub fn sys_umount(path_addr: Vaddr, flags: u64, ctx: &Context) -> Result<SyscallReturn> {
    let path = ctx.user_space().read_cstring(path_addr, MAX_FILENAME_LEN)?;
    let umount_flags = UmountFlags::from_bits_truncate(flags as u32);
    debug!("path = {:?}, flags = {:?}", path, umount_flags);

    umount_flags.check_unsupported_flags()?;

    let path = path.to_string_lossy();
    if path.is_empty() {
        return_errno_with_message!(Errno::ENOENT, "path is empty");
    }
    let fs_path = FsPath::new(AT_FDCWD, path.as_ref())?;

    let target_dentry = if umount_flags.contains(UmountFlags::UMOUNT_NOFOLLOW) {
        ctx.posix_thread
            .fs()
            .resolver()
            .read()
            .lookup_no_follow(&fs_path)?
    } else {
        ctx.posix_thread.fs().resolver().read().lookup(&fs_path)?
    };

    target_dentry.unmount()?;

    Ok(SyscallReturn::Return(0))
}

bitflags! {
    struct UmountFlags: u32 {
        const MNT_FORCE       = 0x00000001;	// Attempt to forcibily umount.
        const MNT_DETACH      = 0x00000002;	// Just detach from the tree.
        const MNT_EXPIRE      = 0x00000004;	// Mark for expiry.
        const UMOUNT_NOFOLLOW = 0x00000008;	// Don't follow symlink on umount.
    }
}

impl UmountFlags {
    fn check_unsupported_flags(&self) -> Result<()> {
        let supported_flags = UmountFlags::MNT_FORCE
            | UmountFlags::MNT_DETACH
            | UmountFlags::MNT_EXPIRE
            | UmountFlags::UMOUNT_NOFOLLOW;
        let unsupported_flags = *self - supported_flags;
        if !unsupported_flags.is_empty() {
            return_errno_with_message!(Errno::EINVAL, "unsupported flags");
        }
        Ok(())
    }
}
