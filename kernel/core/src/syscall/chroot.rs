// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{fs::vfs::path::FsPath, prelude::*, syscall::constants::MAX_FILENAME_LEN};

pub(super) fn sys_chroot(path_ptr: Vaddr, ctx: &Context) -> Result<SyscallReturn> {
    let path_name = ctx.user_space().read_cstring(path_ptr, MAX_FILENAME_LEN)?;
    debug!("path_name = {:?}", path_name);

    let fs_ref = ctx.thread_local.borrow_fs();
    let mut path_resolver = fs_ref.resolver().write();
    let path = {
        let path_name = path_name.to_string_lossy();
        let fs_path = FsPath::try_from(path_name.as_ref())?;
        path_resolver.lookup(&fs_path)?
    };
    path_resolver.chroot(path, ctx)?;
    Ok(SyscallReturn::Return(0))
}
