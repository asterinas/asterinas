// SPDX-License-Identifier: MPL-2.0

use super::super::SyscallReturn;
use crate::{
    fs::{
        file::{
            DetachedMountFile, FileLike, FsConfigFile,
            file_table::{FdFlags, RawFileDesc, get_file_fast},
        },
        vfs::path::{Mount, PerMountFlags},
    },
    prelude::*,
};

pub fn sys_fsmount(
    fs_fd: RawFileDesc,
    flags: u32,
    mount_attrs: u32,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let flags = FsMountFlags::from_bits(flags)
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "unknown fsmount flags"))?;
    let per_mount_flags = mount_attrs_to_per_mount_flags(mount_attrs)?;
    let (fs, source) = {
        let mut file_table = ctx.thread_local.borrow_file_table_mut();
        let file = get_file_fast!(&mut file_table, fs_fd.try_into()?);
        let fs_config = file
            .downcast_ref::<FsConfigFile>()
            .ok_or_else(|| Error::with_message(Errno::EINVAL, "the file is not a fs context"))?;
        (fs_config.created_fs()?, fs_config.source())
    };

    let current_ns_proxy = ctx.thread_local.borrow_ns_proxy();
    let current_mnt_ns = current_ns_proxy.unwrap().mnt_ns();
    let detached_mount = Mount::new_detached(
        fs.clone(),
        per_mount_flags,
        Arc::downgrade(current_mnt_ns),
        source,
    )?;
    let file = Arc::new(DetachedMountFile::new(detached_mount)) as Arc<dyn FileLike>;
    let fd_flags = if flags.contains(FsMountFlags::FSMOUNT_CLOEXEC) {
        FdFlags::CLOEXEC
    } else {
        FdFlags::empty()
    };
    let fd = ctx
        .thread_local
        .borrow_file_table()
        .unwrap()
        .write()
        .insert(file, fd_flags);
    Ok(SyscallReturn::Return(fd.into()))
}

fn mount_attrs_to_per_mount_flags(attrs: u32) -> Result<PerMountFlags> {
    let attrs = MountAttrs::from_bits(attrs)
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "unsupported mount attributes"))?;
    let atime_attrs =
        attrs & (MountAttrs::RELATIME | MountAttrs::NOATIME | MountAttrs::STRICTATIME);
    if atime_attrs.bits().count_ones() > 1 {
        return_errno_with_message!(Errno::EINVAL, "conflicting atime mount attributes");
    }

    let mut flags = if atime_attrs.is_empty() {
        PerMountFlags::default()
    } else {
        PerMountFlags::empty()
    };
    if attrs.contains(MountAttrs::RDONLY) {
        flags |= PerMountFlags::RDONLY;
    }
    if attrs.contains(MountAttrs::NOSUID) {
        flags |= PerMountFlags::NOSUID;
    }
    if attrs.contains(MountAttrs::NODEV) {
        flags |= PerMountFlags::NODEV;
    }
    if attrs.contains(MountAttrs::NOEXEC) {
        flags |= PerMountFlags::NOEXEC;
    }
    if attrs.contains(MountAttrs::RELATIME) {
        flags |= PerMountFlags::RELATIME;
    }
    if attrs.contains(MountAttrs::NOATIME) {
        flags |= PerMountFlags::NOATIME;
    }
    if attrs.contains(MountAttrs::STRICTATIME) {
        flags |= PerMountFlags::STRICTATIME;
    }
    if attrs.contains(MountAttrs::NODIRATIME) {
        flags |= PerMountFlags::NODIRATIME;
    }

    Ok(flags)
}

bitflags! {
    struct FsMountFlags: u32 {
        const FSMOUNT_CLOEXEC = 1;
    }

    struct MountAttrs: u32 {
        const RDONLY = 0x0000_0001;
        const NOSUID = 0x0000_0002;
        const NODEV = 0x0000_0004;
        const NOEXEC = 0x0000_0008;
        const NOATIME = 0x0000_0010;
        const STRICTATIME = 0x0000_0020;
        const RELATIME = 0x0000_0040;
        const NODIRATIME = 0x0000_0080;
    }
}
