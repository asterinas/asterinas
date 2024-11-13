// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::fs_resolver::{FsPath, AT_FDCWD},
    prelude::*,
};

pub fn sys_getcwd(buf: Vaddr, len: usize, ctx: &Context) -> Result<SyscallReturn> {
    let current = ctx.process;
    let dirent = current
        .fs()
        .read()
        .lookup(&FsPath::new(AT_FDCWD, "").unwrap())
        .unwrap();
    let name = dirent.abs_path();

    debug!("getcwd: {:?}", name);

    let cwd = CString::new(name)?;
    let bytes = cwd.as_bytes_with_nul();
    let write_len = len.min(bytes.len());
    ctx.user_space()
        .write_bytes(buf, &mut VmReader::from(&bytes[..write_len]))?;

    Ok(SyscallReturn::Return(write_len as _))
}
