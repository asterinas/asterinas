// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::prelude::*;

pub(super) fn sys_mqnotify(mqdes: i32, notification: Vaddr, ctx: &Context) -> Result<SyscallReturn> {
    // TODO: Implement `mq_notify`.
    let _ = (mqdes, notification, ctx);
    todo!("implement sys_mqnotify")
}
