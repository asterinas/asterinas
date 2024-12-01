// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::{fs_resolver::FsPath, utils::InodeType},
    prelude::*,
    syscall::constants::MAX_FILENAME_LEN,
};

pub fn sys_chroot(path_ptr: Vaddr, ctx: &Context) -> Result<SyscallReturn> {
    let path = ctx.user_space().read_cstring(path_ptr, MAX_FILENAME_LEN)?;
    debug!("path = {:?}", path);

    let mut fs = ctx.posix_thread.fs().resolver().write();
    let dentry = {
        let path = path.to_string_lossy();
        if path.is_empty() {
            return_errno_with_message!(Errno::ENOENT, "path is empty");
        }
        let fs_path = FsPath::try_from(path.as_ref())?;
        fs.lookup(&fs_path)?
    };
    if dentry.type_() != InodeType::Dir {
        return_errno_with_message!(Errno::ENOTDIR, "must be directory");
    }
    fs.set_root(dentry);
    Ok(SyscallReturn::Return(0))
}
