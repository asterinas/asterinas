// SPDX-License-Identifier: MPL-2.0

use super::{sched_getattr::access_sched_attr_with, SyscallReturn};
use crate::{prelude::*, sched::SchedPolicy, thread::Tid};

pub fn sys_sched_getparam(tid: Tid, addr: Vaddr, ctx: &Context) -> Result<SyscallReturn> {
    let policy = access_sched_attr_with(tid, ctx, |attr| Ok(attr.policy()))?;
    let rt_prio = match policy {
        SchedPolicy::RealTime { rt_prio, .. } => rt_prio.get().into(),
        _ => 0,
    };

    let space = ctx.user_space();
    space
        .write_val(addr, &rt_prio)
        .map_err(|_| Error::new(Errno::EINVAL))?;

    Ok(SyscallReturn::Return(0))
}
