// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{fs::fs_resolver::FsPath, prelude::*, syscall::constants::MAX_FILENAME_LEN};

pub fn sys_pivot_root(
    new_root_ptr: Vaddr,
    put_old_ptr: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let new_root_name = ctx
        .user_space()
        .read_cstring(new_root_ptr, MAX_FILENAME_LEN)?;
    let put_old_name = ctx
        .user_space()
        .read_cstring(put_old_ptr, MAX_FILENAME_LEN)?;

    debug!(
        "pivot_root: new_root: {:?}, put_old: {:?}",
        new_root_name, put_old_name
    );

    let (new_root_path, put_old_path) = {
        let fs_ref = ctx.thread_local.borrow_fs();
        let fs_resolver = fs_ref.resolver().read();

        let new_root_name = new_root_name.to_string_lossy();
        let new_root_path = FsPath::try_from(new_root_name.as_ref())?;

        let put_old_name = put_old_name.to_string_lossy();
        let put_old_path = FsPath::try_from(put_old_name.as_ref())?;

        (
            fs_resolver.lookup(&new_root_path)?,
            fs_resolver.lookup(&put_old_path)?,
        )
    };

    new_root_path.pivot_root(&put_old_path, ctx)?;

    Ok(SyscallReturn::Return(0))
}
