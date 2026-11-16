// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::prelude::*;

pub(super) fn sys_mqunlink(name: Vaddr, ctx: &Context) -> Result<SyscallReturn> {
    // TODO: Implement `mq_unlink`.
    let _ = (name, ctx);
    todo!("implement sys_mqunlink")
}
