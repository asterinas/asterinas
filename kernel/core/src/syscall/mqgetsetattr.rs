// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::prelude::*;

pub(super) fn sys_mqgetsetattr(
    mqdes: i32,
    mqstat: Vaddr,
    omqstat: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    // TODO: Implement `mq_getsetattr`.
    let _ = (mqdes, mqstat, omqstat, ctx);
    todo!("implement sys_mqgetsetattr")
}
