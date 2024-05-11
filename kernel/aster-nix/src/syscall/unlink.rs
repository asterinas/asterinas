// SPDX-License-Identifier: MPL-2.0

use super::{SyscallReturn, SYS_UNLINKAT};
use crate::{
    fs::{
        file_table::FileDesc,
        fs_resolver::{FsPath, AT_FDCWD},
    },
    log_syscall_entry,
    prelude::*,
    syscall::constants::MAX_FILENAME_LEN,
    util::read_cstring_from_user,
};

pub fn sys_unlinkat(dirfd: FileDesc, path_addr: Vaddr, flags: u32) -> Result<SyscallReturn> {
    let flags =
        UnlinkFlags::from_bits(flags).ok_or(Error::with_message(Errno::EINVAL, "invalid flags"))?;
    if flags.contains(UnlinkFlags::AT_REMOVEDIR) {
        return super::rmdir::sys_rmdirat(dirfd, path_addr);
    }

    log_syscall_entry!(SYS_UNLINKAT);
    let path = read_cstring_from_user(path_addr, MAX_FILENAME_LEN)?;
    debug!("dirfd = {}, path = {:?}", dirfd, path);

    let current = current!();
    let (dir_dentrymnt, name) = {
        let path = path.to_string_lossy();
        if path.is_empty() {
            return_errno_with_message!(Errno::ENOENT, "path is empty");
        }
        if path.ends_with('/') {
            return_errno_with_message!(Errno::EISDIR, "unlink on directory");
        }
        let fs_path = FsPath::new(dirfd, path.as_ref())?;
        current.fs().read().lookup_dir_and_base_name(&fs_path)?
    };
    dir_dentrymnt.unlink(&name)?;
    Ok(SyscallReturn::Return(0))
}

pub fn sys_unlink(path_addr: Vaddr) -> Result<SyscallReturn> {
    self::sys_unlinkat(AT_FDCWD, path_addr, 0)
}

bitflags::bitflags! {
    struct UnlinkFlags: u32 {
        const AT_REMOVEDIR = 0x200;
    }
}
