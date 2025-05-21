// SPDX-License-Identifier: MPL-2.0

use super::{
    sched_getattr::{access_sched_attr_with, read_linux_sched_attr_from_user},
    SyscallReturn,
};
use crate::{prelude::*, sched::SchedPolicy, thread::Tid};

pub fn sys_sched_setattr(
    tid: Tid,
    addr: Vaddr,
    flags: u32,
    ctx: &Context,
) -> Result<SyscallReturn> {
    if flags != 0 {
        // TODO: support flags soch as `RESET_ON_FORK`.
        return Err(Error::with_message(Errno::EINVAL, "unsupported flags"));
    }

    let attr = read_linux_sched_attr_from_user(addr, ctx).map_err(|_| Error::new(Errno::EINVAL))?;
    let policy = SchedPolicy::try_from(attr)?;
    access_sched_attr_with(tid, ctx, |attr| {
        attr.set_policy(policy);
        Ok(())
    })?;

    Ok(SyscallReturn::Return(0))
}
