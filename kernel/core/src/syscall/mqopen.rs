// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::prelude::*;

pub(super) fn sys_mqopen(
    name: Vaddr,
    oflag: i32,
    mode: u16,
    attr: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    // TODO: Implement `mq_open`.
    let _ = (name, oflag, mode, attr, ctx);
    todo!("implement sys_mqopen")
}
