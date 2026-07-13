// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::{file::InodeType, vfs::path::FsPath},
    prelude::*,
    process::credentials::capabilities::CapSet,
    security::lsm::hooks as lsm_hooks,
    syscall::constants::MAX_FILENAME_LEN,
};

pub fn sys_chroot(path_ptr: Vaddr, ctx: &Context) -> Result<SyscallReturn> {
    let path_name = ctx.user_space().read_cstring(path_ptr, MAX_FILENAME_LEN)?;
    debug!("path_name = {:?}", path_name);

    let fs_ref = ctx.thread_local.borrow_fs();
    let mut path_resolver = fs_ref.resolver().write();
    let path = {
        let path_name = path_name.to_string_lossy();
        let fs_path = FsPath::try_from(path_name.as_ref())?;
        path_resolver.lookup(&fs_path)?
    };

    if path.type_() != InodeType::Dir {
        return_errno_with_message!(Errno::ENOTDIR, "must be directory");
    }

    lsm_hooks::on_capable(lsm_hooks::CapableContext::new(
        ctx.thread_local.borrow_user_ns().as_ref(),
        ctx.posix_thread,
        CapSet::SYS_CHROOT,
    ))?;

    path_resolver.set_root(path);
    Ok(SyscallReturn::Return(0))
}
