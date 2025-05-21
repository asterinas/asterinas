// SPDX-License-Identifier: MPL-2.0

use super::{sched_get_priority_max::SCHED_PRIORITY_RANGE, SyscallReturn};
use crate::prelude::*;

pub fn sys_sched_get_priority_min(policy: u32, _: &Context) -> Result<SyscallReturn> {
    let range = SCHED_PRIORITY_RANGE
        .get(policy as usize)
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "invalid scheduling policy"))?;
    Ok(SyscallReturn::Return(*range.start() as isize))
}
