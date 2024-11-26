// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{net::HOSTNAME, prelude::*};

/// SUSv2 guarantees that "Host names are limited to 255 bytes".
/// POSIX.1 guarantees that "Host names (not including the
/// terminating null byte) are limited to HOST_NAME_MAX bytes".  On
/// Linux, HOST_NAME_MAX is defined with the value 64, which has been
/// the limit since Linux 1.0 (earlier kernels imposed a limit of 8
/// bytes).
const HOST_NAME_MAX: usize = 64;

pub fn sys_sethostname(name: Vaddr, len: usize, ctx: &Context) -> Result<SyscallReturn> {
    if len == 0 {
        return Err(Error::new(Errno::EINVAL))
    }
    if len > HOST_NAME_MAX {
        return Err(Error::new(Errno::ENAMETOOLONG));
    }

    let mut buf = vec![0u8; len];
    ctx.user_space()
        .read_bytes(name, &mut VmWriter::from(buf.as_mut_slice()))?;
    // name does not contain a null-terminator
    let name = CString::new(buf)?;
    let mut hostname = HOSTNAME.get().unwrap().write();
    *hostname = name;
    Ok(SyscallReturn::Return(0))
}
