// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{prelude::*, security::seccomp};

pub(super) fn sys_seccomp(
    operation: u32,
    flags: u32,
    args_addr: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let result = seccomp::seccomp(operation, flags, args_addr, ctx)?;
    Ok(SyscallReturn::Return(result))
}
