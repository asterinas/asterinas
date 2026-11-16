// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::prelude::*;

pub(super) fn sys_mqtimedsend(
    mqdes: i32,
    msg_ptr: Vaddr,
    msg_len: usize,
    msg_prio: u32,
    abs_timeout: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    // TODO: Implement `mq_timedsend`.
    let _ = (mqdes, msg_ptr, msg_len, msg_prio, abs_timeout, ctx);
    todo!("implement sys_mqtimedsend")
}
