use crate::fs::{
    file_table::FileDescripter,
    fs_resolver::{FsPath, AT_FDCWD},
};
use crate::log_syscall_entry;
use crate::prelude::*;
use crate::syscall::constants::MAX_FILENAME_LEN;
use crate::util::read_cstring_from_user;

use super::SyscallReturn;
use super::SYS_UNLINKAT;

pub fn sys_unlinkat(
    dirfd: FileDescripter,
    pathname_addr: Vaddr,
    flags: u32,
) -> Result<SyscallReturn> {
    let flags =
        UnlinkFlags::from_bits(flags).ok_or(Error::with_message(Errno::EINVAL, "invalid flags"))?;
    if flags.contains(UnlinkFlags::AT_REMOVEDIR) {
        return super::rmdir::sys_rmdirat(dirfd, pathname_addr);
    }

    log_syscall_entry!(SYS_UNLINKAT);
    let pathname = read_cstring_from_user(pathname_addr, MAX_FILENAME_LEN)?;
    debug!("dirfd = {}, pathname = {:?}", dirfd, pathname);

    let current = current!();
    let (dir_dentry, name) = {
        let pathname = pathname.to_string_lossy();
        if pathname.is_empty() {
            return_errno_with_message!(Errno::ENOENT, "path is empty");
        }
        if pathname.ends_with('/') {
            return_errno_with_message!(Errno::EISDIR, "unlink on directory");
        }
        let fs_path = FsPath::new(dirfd, pathname.as_ref())?;
        current.fs().read().lookup_dir_and_base_name(&fs_path)?
    };
    dir_dentry.unlink(&name)?;
    Ok(SyscallReturn::Return(0))
}

pub fn sys_unlink(pathname_addr: Vaddr) -> Result<SyscallReturn> {
    self::sys_unlinkat(AT_FDCWD, pathname_addr, 0)
}

bitflags::bitflags! {
    struct UnlinkFlags: u32 {
        const AT_REMOVEDIR = 0x200;
    }
}
