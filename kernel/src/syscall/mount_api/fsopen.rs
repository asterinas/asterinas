// SPDX-License-Identifier: MPL-2.0

use super::super::{SyscallReturn, constants::MAX_FILENAME_LEN};
use crate::{
    fs::{
        file::{FileLike, FsConfigFile, file_table::FdFlags},
        vfs::registry::look_up,
    },
    prelude::*,
};

pub fn sys_fsopen(fs_name_addr: Vaddr, flags: u32, ctx: &Context) -> Result<SyscallReturn> {
    if fs_name_addr == 0 {
        return_errno_with_message!(Errno::EINVAL, "fsopen filesystem name is NULL");
    }

    let flags = FsOpenFlags::from_bits(flags)
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "unknown fsopen flags"))?;
    let fs_name = ctx
        .user_space()
        .read_cstring(fs_name_addr, MAX_FILENAME_LEN)?;
    let fs_name = fs_name
        .to_str()
        .map_err(|_| Error::with_message(Errno::EINVAL, "invalid file system name"))?;
    if look_up(fs_name).is_none() {
        return_errno_with_message!(
            Errno::ENODEV,
            "the filesystem is not configured in the kernel"
        );
    }

    let file = Arc::new(FsConfigFile::new()) as Arc<dyn FileLike>;
    let fd_flags = if flags.contains(FsOpenFlags::FSOPEN_CLOEXEC) {
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

bitflags! {
    struct FsOpenFlags: u32 {
        const FSOPEN_CLOEXEC = 1;
    }
}
