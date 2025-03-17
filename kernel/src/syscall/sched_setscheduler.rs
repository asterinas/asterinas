// SPDX-License-Identifier: MPL-2.0

use super::{
    sched_getattr::{access_sched_attr_with, LinuxSchedAttr},
    SyscallReturn,
};
use crate::{prelude::*, thread::Tid};

pub fn sys_sched_setscheduler(
    tid: Tid,
    policy: i32,
    addr: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let space = ctx.user_space();
    let prio = space
        .read_val(addr)
        .map_err(|_| Error::new(Errno::EINVAL))?;

    let attr = LinuxSchedAttr {
        sched_policy: policy as u32,
        sched_priority: prio,
        ..Default::default()
    };

    let policy = attr.try_into()?;
    access_sched_attr_with(tid, ctx, |attr| {
        attr.set_policy(policy);
        Ok(())
    })?;

    Ok(SyscallReturn::Return(0))
}
