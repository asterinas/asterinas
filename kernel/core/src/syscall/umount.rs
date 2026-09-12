// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::vfs::path::{AT_FDCWD, EmptyPathStr, FsPath, UnmountMode},
    prelude::*,
    syscall::constants::MAX_FILENAME_LEN,
};

pub(super) fn sys_umount(path_addr: Vaddr, flags: u64, ctx: &Context) -> Result<SyscallReturn> {
    let path_name = ctx.user_space().read_cstring(path_addr, MAX_FILENAME_LEN)?;
    let umount_flags = UmountFlags::from_bits_truncate(flags as u32);
    debug!("path = {:?}, flags = {:?}", path_name, umount_flags);

    umount_flags.check_compatible_flags()?;

    let path_name = path_name.to_string_lossy();
    let fs_path = FsPath::from_fd_at(AT_FDCWD, &path_name, EmptyPathStr::Reject)?;

    let target_path = if umount_flags.contains(UmountFlags::UMOUNT_NOFOLLOW) {
        ctx.thread_local
            .borrow_fs()
            .resolver()
            .read()
            .lookup_no_follow(&fs_path)?
    } else {
        ctx.thread_local
            .borrow_fs()
            .resolver()
            .read()
            .lookup(&fs_path)?
    };

    // The path resolution of the umount syscall ensures that the final `Path` must correspond
    // to the topmost mount. If there is a mount stacked above the current thread's `cwd`, normal
    // path lookup through "." cannot access the upper mount, but umount through "." can operate
    // on the upper mount.
    //
    // `MNT_FORCE` and `MNT_EXPIRE` are accepted as flags but do not yet implement
    // full Linux semantics. Only `MNT_DETACH` selects a distinct topology mode.
    let mode = if umount_flags.contains(UmountFlags::MNT_DETACH) {
        UnmountMode::Detach
    } else {
        UnmountMode::Regular
    };
    target_path.get_top_path().unmount(mode, ctx)?;

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
    fn check_compatible_flags(&self) -> Result<()> {
        if self.contains(UmountFlags::MNT_EXPIRE)
            && self.intersects(UmountFlags::MNT_DETACH | UmountFlags::MNT_FORCE)
        {
            return_errno_with_message!(
                Errno::EINVAL,
                "MNT_EXPIRE cannot be combined with MNT_DETACH or MNT_FORCE"
            );
        }
        Ok(())
    }
}
