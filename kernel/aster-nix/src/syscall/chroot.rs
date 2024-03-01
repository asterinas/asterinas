// SPDX-License-Identifier: MPL-2.0

use super::{SyscallReturn, SYS_CHROOT};
use crate::{
    fs::{fs_resolver::FsPath, utils::InodeType},
    log_syscall_entry,
    prelude::*,
    syscall::constants::MAX_FILENAME_LEN,
    util::read_cstring_from_user,
};

pub fn sys_chroot(pathname_addr: Vaddr) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_CHROOT);
    let pathname = read_cstring_from_user(pathname_addr, MAX_FILENAME_LEN)?;
    debug!("pathname = {:?}", pathname);

    let current = current!();
    let mut fs = current.fs().write();
    let dentry = {
        let pathname = pathname.to_string_lossy();
        if pathname.is_empty() {
            return_errno_with_message!(Errno::ENOENT, "path is empty");
        }
        let fs_path = FsPath::try_from(pathname.as_ref())?;
        fs.lookup(&fs_path)?
    };
    if dentry.type_() != InodeType::Dir {
        return_errno_with_message!(Errno::ENOTDIR, "must be directory");
    }
    fs.set_root(dentry);
    Ok(SyscallReturn::Return(0))
}
