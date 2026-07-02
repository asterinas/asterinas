// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::{
        file::{
            DetachedMountFile, FileLike, FsConfigFile,
            file_table::{FdFlags, RawFileDesc, get_file_fast},
        },
        vfs::path::PerMountFlags,
    },
    prelude::*,
};

pub fn sys_fsmount(
    fs_fd: RawFileDesc,
    flags: u32,
    mount_attrs: u32,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let flags = FsMountFlags::try_from(flags)?;
    let mount_attrs = MountAttrs::try_from(mount_attrs)?;
    let per_mount_flags = PerMountFlags::try_from(mount_attrs)?;
    super::fsopen::check_mount_api_capability(ctx)?;

    let fs_config_file = {
        let mut file_table = ctx.thread_local.borrow_file_table_mut();
        let file = get_file_fast!(&mut file_table, fs_fd.try_into()?).into_owned();
        file.downcast_ref::<FsConfigFile>()
            .ok_or_else(|| Error::with_message(Errno::EINVAL, "the file is not a fs context"))?;
        file
    };
    let fs_config = fs_config_file
        .downcast_ref::<FsConfigFile>()
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "the file is not a fs context"))?;

    let current_ns_proxy = ctx.thread_local.borrow_ns_proxy();
    let current_mnt_ns = current_ns_proxy.unwrap().mnt_ns();
    let detached_mount =
        fs_config.create_detached_mount(per_mount_flags, Arc::downgrade(current_mnt_ns))?;
    let file = Arc::new(DetachedMountFile::new(detached_mount)) as Arc<dyn FileLike>;
    let fd = ctx
        .thread_local
        .borrow_file_table()
        .unwrap()
        .write()
        .insert(file, FdFlags::from(flags));
    Ok(SyscallReturn::Return(fd.into()))
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
        const NODIRATIME = 0x0000_0080;
        const NOSYMFOLLOW = 0x0020_0000;
    }
}

impl TryFrom<u32> for FsMountFlags {
    type Error = Error;

    fn try_from(value: u32) -> Result<Self> {
        Self::from_bits(value)
            .ok_or_else(|| Error::with_message(Errno::EINVAL, "unknown fsmount flags"))
    }
}

impl From<FsMountFlags> for FdFlags {
    fn from(value: FsMountFlags) -> Self {
        if value.contains(FsMountFlags::FSMOUNT_CLOEXEC) {
            Self::CLOEXEC
        } else {
            Self::empty()
        }
    }
}

impl TryFrom<u32> for MountAttrs {
    type Error = Error;

    fn try_from(value: u32) -> Result<Self> {
        Self::from_bits(value)
            .ok_or_else(|| Error::with_message(Errno::EINVAL, "unsupported mount attributes"))
    }
}

impl TryFrom<MountAttrs> for PerMountFlags {
    type Error = Error;

    fn try_from(attrs: MountAttrs) -> Result<Self> {
        const SHARED_BITS: MountAttrs = MountAttrs::RDONLY
            .union(MountAttrs::NOSUID)
            .union(MountAttrs::NODEV)
            .union(MountAttrs::NOEXEC);

        let atime_attrs = attrs & (MountAttrs::NOATIME | MountAttrs::STRICTATIME);
        if atime_attrs.bits().count_ones() > 1 {
            return_errno_with_message!(Errno::EINVAL, "conflicting atime mount attributes");
        }

        let mut flags = PerMountFlags::from_bits_truncate((attrs & SHARED_BITS).bits());
        if atime_attrs.is_empty() {
            flags |= PerMountFlags::RELATIME;
        } else if attrs.contains(MountAttrs::NOATIME) {
            flags |= PerMountFlags::NOATIME;
        } else {
            flags |= PerMountFlags::STRICTATIME;
        }
        if attrs.contains(MountAttrs::NODIRATIME) {
            flags |= PerMountFlags::NODIRATIME;
        }
        if attrs.contains(MountAttrs::NOSYMFOLLOW) {
            flags |= PerMountFlags::NOSYMFOLLOW;
        }

        Ok(flags)
    }
}
