// SPDX-License-Identifier: MPL-2.0

use ostd::mm::VmIo;

use super::SyscallReturn;
use crate::{fs::vfs::path::AbsPathResult, prelude::*};

pub fn sys_getcwd(buf: Vaddr, len: usize, ctx: &Context) -> Result<SyscallReturn> {
    let abs_path = {
        let fs_ref = ctx.thread_local.borrow_fs();
        let path_resolver = fs_ref.resolver().read();
        path_resolver.make_abs_path(path_resolver.cwd())?
    };

    debug!("getcwd: {:?}", abs_path);

    // Linux will add a "(unreachable)" prefix to the path if the CWD is not reachable.
    // However, according to POSIX, `getcwd()` should fail with ENOENT in this case.
    // `glibc` treats the Linux's behavior as a bug and handles the Linux-specific prefix
    // to conform to POSIX. Here follows Linux's behavior to keep compatibility.
    //
    // Reference: <https://man7.org/linux/man-pages/man3/getcwd.3.html>
    let path_buf = match abs_path {
        AbsPathResult::Reachable(buf) => buf,
        AbsPathResult::Unreachable(mut buf) => {
            buf.prepend_bytes(b"(unreachable)");
            buf
        }
    };

    let path_bytes = path_buf.as_bytes();
    let total_len = path_bytes.len() + 1;
    if total_len > len {
        return_errno_with_message!(Errno::ERANGE, "the CWD buffer is too small");
    }
    ctx.user_space().write_bytes(buf, path_bytes)?;
    ctx.user_space().write_bytes(buf + path_bytes.len(), &[0])?;

    Ok(SyscallReturn::Return(total_len as _))
}
