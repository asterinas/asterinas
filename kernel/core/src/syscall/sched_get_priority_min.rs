// SPDX-License-Identifier: MPL-2.0

use super::{SyscallReturn, sched_get_priority_max::sched_priority_range};
use crate::{prelude::*, sched::LinuxSchedPolicy};

pub fn sys_sched_get_priority_min(policy: u32, _: &Context) -> Result<SyscallReturn> {
    let linux_policy = LinuxSchedPolicy::try_from(policy)?;
    let range = sched_priority_range(linux_policy);
    Ok(SyscallReturn::Return(*range.start() as isize))
}
