// SPDX-License-Identifier: MPL-2.0

use ostd::mm::VmIo;

use super::{sched_getattr::access_sched_attr_with, SyscallReturn};
use crate::{prelude::*, sched::SchedPolicy, thread::Tid};

pub fn sys_sched_setparam(tid: Tid, addr: Vaddr, ctx: &Context) -> Result<SyscallReturn> {
    if addr == 0 {
        return_errno_with_message!(Errno::EINVAL, "invalid user space address");
    }

    let prio: i32 = ctx.user_space().read_val(addr)?;

    let update = |policy: &mut SchedPolicy| {
        match policy {
            SchedPolicy::RealTime { rt_prio, .. } => {
                *rt_prio = u8::try_from(prio)
                    .ok()
                    .and_then(|p| p.try_into().ok())
                    .ok_or_else(|| {
                        Error::with_message(Errno::EINVAL, "invalid scheduling priority")
                    })?;
            }
            _ if prio != 0 => {
                return_errno_with_message!(Errno::EINVAL, "invalid scheduling priority")
            }
            _ => {}
        }
        Ok(())
    };
    access_sched_attr_with(tid, ctx, |attr| attr.update_policy(update))?;

    Ok(SyscallReturn::Return(0))
}
