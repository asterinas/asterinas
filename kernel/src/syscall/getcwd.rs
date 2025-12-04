// SPDX-License-Identifier: MPL-2.0

use ostd::mm::VmIo;

use super::SyscallReturn;
use crate::prelude::*;

pub fn sys_getcwd(buf: Vaddr, len: usize, ctx: &Context) -> Result<SyscallReturn> {
    let dirent = ctx.thread_local.borrow_fs().resolver().read().cwd().clone();
    let name = dirent.abs_path();
    debug!("getcwd: {:?}", name);

    let cwd = CString::new(name)?;
    let bytes = cwd.as_bytes_with_nul();
    let write_len = len.min(bytes.len());
    ctx.user_space().write_bytes(buf, &bytes[..write_len])?;

    Ok(SyscallReturn::Return(write_len as _))
}
