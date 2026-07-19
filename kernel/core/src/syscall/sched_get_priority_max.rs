// SPDX-License-Identifier: MPL-2.0

use core::ops::RangeInclusive;

use super::SyscallReturn;
use crate::{
    prelude::*,
    sched::{LinuxSchedPolicy, RealTimePriority},
};

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

pub(super) const fn sched_priority_range(policy: LinuxSchedPolicy) -> RangeInclusive<u32> {
    match policy {
        LinuxSchedPolicy::Fifo | LinuxSchedPolicy::RoundRobin => RT_PRIORITY_RANGE,
        LinuxSchedPolicy::Normal
        | LinuxSchedPolicy::Batch
        | LinuxSchedPolicy::Iso
        | LinuxSchedPolicy::Idle
        | LinuxSchedPolicy::Deadline
        | LinuxSchedPolicy::Ext => 0..=0,
    }
}

pub fn sys_sched_get_priority_max(policy: u32, _: &Context) -> Result<SyscallReturn> {
    let linux_policy = LinuxSchedPolicy::try_from(policy)?;
    let range = sched_priority_range(linux_policy);
    Ok(SyscallReturn::Return(*range.end() as isize))
}
