// SPDX-License-Identifier: MPL-2.0

use crate::log_syscall_entry;
use crate::prelude::*;
use crate::util::write_bytes_to_user;

use super::SyscallReturn;
use super::SYS_GETCWD;

pub fn sys_getcwd(buf: Vaddr, len: usize) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_GETCWD);
    // TODO: getcwd only return a fake result now
    let fake_cwd = CString::new("/")?;
    let bytes = fake_cwd.as_bytes_with_nul();
    let write_len = len.min(bytes.len());
    write_bytes_to_user(buf, &bytes[..write_len])?;
    Ok(SyscallReturn::Return(write_len as _))
}
