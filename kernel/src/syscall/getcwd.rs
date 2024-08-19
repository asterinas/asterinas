// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::prelude::*;

pub fn sys_getcwd(buf: Vaddr, len: usize, ctx: &Context) -> Result<SyscallReturn> {
    // TODO: getcwd only return a fake result now
    let fake_cwd = CString::new("/")?;
    let bytes = fake_cwd.as_bytes_with_nul();
    let write_len = len.min(bytes.len());
    ctx.get_user_space()
        .write_bytes(buf, &mut VmReader::from(&bytes[..write_len]))?;
    Ok(SyscallReturn::Return(write_len as _))
}
