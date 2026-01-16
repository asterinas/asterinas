// SPDX-License-Identifier: MPL-2.0

use ostd::mm::VmIo;

use super::SyscallReturn;
use crate::{fs::path::AbsPathResult, prelude::*};

pub fn sys_getcwd(buf: Vaddr, len: usize, ctx: &Context) -> Result<SyscallReturn> {
    let abs_path = {
        let fs_ref = ctx.thread_local.borrow_fs();
        let path_resolver = fs_ref.resolver().read();
        path_resolver.make_abs_path(path_resolver.cwd())
    };

    debug!("getcwd: {:?}", abs_path);

    // Linux will add a "(unreachable)" prefix to the path if the CWD is not reachable.
    // However, according to POSIX, `getcwd()` should fail with ENOENT in this case.
    // `glibc` treats the Linux's behavior as a bug and handles the Linux-specific prefix
    // to conform to POSIX. Here follows Linux's behavior to keep compatibility.
    //
    // Reference: <https://man7.org/linux/man-pages/man3/getcwd.3.html>
    let abs_path = match abs_path {
        AbsPathResult::Reachable(s) => s,
        AbsPathResult::Unreachable(s) => alloc::format!("(unreachable){}", s),
    };

    let cwd = CString::new(abs_path)?;
    let bytes = cwd.as_bytes_with_nul();
    let write_len = len.min(bytes.len());
    ctx.user_space().write_bytes(buf, &bytes[..write_len])?;

    Ok(SyscallReturn::Return(write_len as _))
}
