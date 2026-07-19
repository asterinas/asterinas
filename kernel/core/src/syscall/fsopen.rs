// SPDX-License-Identifier: MPL-2.0

use super::{SyscallReturn, constants::MAX_FILENAME_LEN};
use crate::{
    fs::{
        file::{FileLike, FsConfigFile, file_table::FdFlags},
        vfs::registry::look_up,
    },
    prelude::*,
};

pub fn sys_fsopen(fs_name_addr: Vaddr, flags: u32, ctx: &Context) -> Result<SyscallReturn> {
    let flags = FsOpenFlags::try_from(flags)?;
    check_mount_api_capability(ctx)?;

    let fs_name = ctx
        .user_space()
        .read_cstring(fs_name_addr, MAX_FILENAME_LEN)?;
    let fs_name = fs_name
        .to_str()
        .map_err(|_| Error::with_message(Errno::EINVAL, "invalid filesystem name"))?;
    let fs_type = look_up(fs_name).ok_or_else(|| {
        Error::with_message(
            Errno::ENODEV,
            "the filesystem is not configured in the kernel",
        )
    })?;

    let file = Arc::new(FsConfigFile::new(fs_type)) as Arc<dyn FileLike>;
    let fd = ctx
        .thread_local
        .borrow_file_table()
        .unwrap()
        .write()
        .insert(file, FdFlags::from(flags));
    Ok(SyscallReturn::Return(fd.into()))
}

pub(super) fn check_mount_api_capability(ctx: &Context) -> Result<()> {
    use crate::{process::credentials::capabilities::CapSet, security::lsm::hooks as lsm_hooks};

    lsm_hooks::on_capable(lsm_hooks::CapableContext::new(
        ctx.thread_local.borrow_user_ns().as_ref(),
        ctx.posix_thread,
        CapSet::SYS_ADMIN,
    ))
}

bitflags! {
    struct FsOpenFlags: u32 {
        const FSOPEN_CLOEXEC = 1;
    }
}

impl TryFrom<u32> for FsOpenFlags {
    type Error = Error;

    fn try_from(value: u32) -> Result<Self> {
        Self::from_bits(value)
            .ok_or_else(|| Error::with_message(Errno::EINVAL, "unknown fsopen flags"))
    }
}

impl From<FsOpenFlags> for FdFlags {
    fn from(value: FsOpenFlags) -> Self {
        if value.contains(FsOpenFlags::FSOPEN_CLOEXEC) {
            Self::CLOEXEC
        } else {
            Self::empty()
        }
    }
}
