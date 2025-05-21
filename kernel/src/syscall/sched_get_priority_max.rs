// SPDX-License-Identifier: MPL-2.0

use core::ops::RangeInclusive;

use super::SyscallReturn;
use crate::{prelude::*, sched::RealTimePriority};

pub(super) const fn rt_to_static(prio: RealTimePriority) -> u32 {
    (100 - prio.get()) as u32
}

pub(super) const fn static_to_rt(prio: u32) -> Result<RealTimePriority> {
    if *RT_PRIORITY_RANGE.start() <= prio && prio <= *RT_PRIORITY_RANGE.end() {
        Ok(RealTimePriority::new((100 - prio) as u8))
    } else {
        Err(Error::with_message(Errno::EINVAL, "invalid priority"))
    }
}

pub(super) const RT_PRIORITY_RANGE: RangeInclusive<u32> =
    rt_to_static(RealTimePriority::MAX)..=rt_to_static(RealTimePriority::MIN);
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
