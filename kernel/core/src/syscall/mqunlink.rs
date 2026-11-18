// SPDX-License-Identifier: MPL-2.0

use super::{SyscallReturn, constants::MAX_FILENAME_LEN};
use crate::{ipc::mqueue::posix::unlink_queue, prelude::*};

pub(super) fn sys_mqunlink(name_addr: Vaddr, ctx: &Context) -> Result<SyscallReturn> {
    let name = ctx.user_space().read_cstring(name_addr, MAX_FILENAME_LEN)?;
    let name = name.to_string_lossy();
    unlink_queue(&name)?;
    Ok(SyscallReturn::Return(0))
}
