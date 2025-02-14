// SPDX-License-Identifier: MPL-2.0

use core::ops::RangeInclusive;

use super::SyscallReturn;
use crate::{prelude::*, sched::RealTimePriority};

pub(super) const RT_PRIORITY_RANGE: RangeInclusive<u32> =
    (RealTimePriority::MIN.get() as u32)..=(RealTimePriority::MAX.get() as u32);
pub(super) const SCHED_PRIORITY_RANGE: &[RangeInclusive<u32>] = &[
    0..=0,             // SCHED_NORMAL
    RT_PRIORITY_RANGE, // SCHED_FIFO
    RT_PRIORITY_RANGE, // SCHED_RR
    0..=0,             // SCHED_BATCH
    0..=0,             // SCHED_ISO
    0..=0,             // SCHED_IDLE
    0..=0,             // SCHED_DEADLINE
    0..=0,             // SCHED_EXT
];

pub fn sys_sched_get_priority_max(policy: u32, _: &Context) -> Result<SyscallReturn> {
    let range = SCHED_PRIORITY_RANGE
        .get(policy as usize)
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "invalid scheduling policy"))?;
    Ok(SyscallReturn::Return(*range.end() as isize))
}
