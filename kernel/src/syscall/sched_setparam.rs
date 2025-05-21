// SPDX-License-Identifier: MPL-2.0

use super::{sched_getattr::access_sched_attr_with, SyscallReturn};
use crate::{prelude::*, sched::SchedPolicy, thread::Tid};

pub fn sys_sched_setparam(tid: Tid, addr: Vaddr, ctx: &Context) -> Result<SyscallReturn> {
    let space = ctx.user_space();
    let prio: i32 = space
        .read_val(addr)
        .map_err(|_| Error::new(Errno::EINVAL))?;

    let update = |policy: &mut SchedPolicy| {
        match policy {
            SchedPolicy::RealTime { rt_prio, .. } => {
                *rt_prio = u8::try_from(prio)?
                    .try_into()
                    .map_err(|msg| Error::with_message(Errno::EINVAL, msg))?;
            }
            _ if prio != 0 => return Err(Error::with_message(Errno::EINVAL, "invalid priority")),
            _ => {}
        }
        Ok(())
    };
    access_sched_attr_with(tid, ctx, |attr| attr.update_policy(update))?;

    Ok(SyscallReturn::Return(0))
}
