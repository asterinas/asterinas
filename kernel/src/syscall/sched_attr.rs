// SPDX-License-Identifier: MPL-2.0

use core::{mem, ops::RangeInclusive};

use super::SyscallReturn;
use crate::{
    prelude::*,
    process::posix_thread::thread_table,
    sched::{Nice, RealTimePolicy, RealTimePriority, SchedAttr, SchedPolicy},
    thread::Tid,
};

const SCHED_NORMAL: u32 = 0;
const SCHED_FIFO: u32 = 1;
const SCHED_RR: u32 = 2;
// const SCHED_BATCH: u32 = 3; // not supported (never).
// SCHED_ISO: reserved but not implemented yet on Linux.
const SCHED_IDLE: u32 = 5;
// const SCHED_DEADLINE: u32 = 6; // not supported yet.
// const SCHED_EXT: u32 = 7; // not supported (never).

const RT_PRIORITY_RANGE: RangeInclusive<u32> =
    (RealTimePriority::MIN.get() as u32)..=(RealTimePriority::MAX.get() as u32);
const SCHED_PRIORITY_RANGE: &[RangeInclusive<u32>] = &[
    0..=0,             // SCHED_NORMAL
    RT_PRIORITY_RANGE, // SCHED_FIFO
    RT_PRIORITY_RANGE, // SCHED_RR
    0..=0,             // SCHED_BATCH
    0..=0,             // SCHED_ISO
    0..=0,             // SCHED_IDLE
    0..=0,             // SCHED_DEADLINE
    0..=0,             // SCHED_EXT
];

#[derive(Default, Debug, Pod, Clone, Copy)]
#[repr(C)]
struct LinuxSchedAttr {
    // Size of this structure
    size: u32,

    // Policy (SCHED_*)
    sched_policy: u32,
    // Flags
    sched_flags: u64,

    // Nice value (SCHED_NORMAL, SCHED_BATCH)
    sched_nice: i32,

    // Static priority (SCHED_FIFO, SCHED_RR)
    sched_priority: u32,

    // For SCHED_DEADLINE
    sched_runtime: u64,
    sched_deadline: u64,
    sched_period: u64,

    // Utilization hints
    sched_util_min: u32,
    sched_util_max: u32,
}

impl TryFrom<SchedPolicy> for LinuxSchedAttr {
    type Error = Error;

    fn try_from(value: SchedPolicy) -> Result<Self> {
        Ok(match value {
            SchedPolicy::Stop => return Err(Error::new(Errno::EACCES)),

            SchedPolicy::RealTime { rt_prio, rt_policy } => LinuxSchedAttr {
                sched_policy: match rt_policy {
                    RealTimePolicy::Fifo => SCHED_FIFO,
                    RealTimePolicy::RoundRobin { .. } => SCHED_RR,
                },
                sched_priority: u32::from(rt_prio.get()),
                ..Default::default()
            },

            SchedPolicy::Fair(nice) => LinuxSchedAttr {
                sched_policy: SCHED_NORMAL,
                sched_nice: i32::from(nice.value().get()),
                ..Default::default()
            },

            SchedPolicy::Idle => LinuxSchedAttr {
                sched_policy: SCHED_IDLE,
                ..Default::default()
            },
        })
    }
}

impl TryFrom<LinuxSchedAttr> for SchedPolicy {
    type Error = Error;

    fn try_from(value: LinuxSchedAttr) -> Result<Self> {
        Ok(match value.sched_policy {
            SCHED_FIFO | SCHED_RR => SchedPolicy::RealTime {
                rt_prio: u8::try_from(value.sched_priority)?
                    .try_into()
                    .map_err(|msg| Error::with_message(Errno::EINVAL, msg))?,
                rt_policy: match value.sched_policy {
                    SCHED_FIFO => RealTimePolicy::Fifo,
                    SCHED_RR => RealTimePolicy::RoundRobin {
                        base_slice_factor: None,
                    },
                    _ => unreachable!(),
                },
            },

            _ if value.sched_priority != 0 => {
                return Err(Error::with_message(
                    Errno::EINVAL,
                    "Invalid scheduling priority",
                ))
            }

            SCHED_NORMAL => SchedPolicy::Fair(Nice::new(
                i8::try_from(value.sched_nice)?
                    .try_into()
                    .map_err(|msg| Error::with_message(Errno::EINVAL, msg))?,
            )),

            SCHED_IDLE => SchedPolicy::Idle,

            _ => {
                return Err(Error::with_message(
                    Errno::EINVAL,
                    "Invalid scheduling policy",
                ))
            }
        })
    }
}

impl LinuxSchedAttr {
    fn write_to_user(mut self, addr: Vaddr, user_size: u32, ctx: &Context) -> Result<()> {
        let space = CurrentUserSpace::new(ctx.task);

        self.size = (mem::size_of::<LinuxSchedAttr>() as u32).min(user_size);

        let range = SCHED_PRIORITY_RANGE
            .get(self.sched_policy as usize)
            .ok_or_else(|| Error::with_message(Errno::EINVAL, "invalid scheduling policy"))?;
        self.sched_util_min = *range.start();
        self.sched_util_max = *range.end();

        space.write_bytes(
            addr,
            &mut VmReader::from(&self.as_bytes()[..self.size as usize]),
        )?;
        Ok(())
    }
}

fn access_sched_attr_with<T>(
    tid: Tid,
    ctx: &Context,
    f: impl FnOnce(&SchedAttr) -> Result<T>,
) -> Result<T> {
    match tid {
        0 => f(&ctx.thread.sched_attr()),
        _ => f(&thread_table::get_thread(tid)
            .ok_or_else(|| Error::with_message(Errno::ESRCH, "thread does not exist"))?
            .sched_attr()),
    }
}

pub fn sys_sched_getattr(
    tid: Tid,
    addr: Vaddr,
    user_size: u32,
    flags: u32,
    ctx: &Context,
) -> Result<SyscallReturn> {
    if flags != 0 {
        // TODO: support flags soch as `RESET_ON_FORK`.
        return Err(Error::with_message(Errno::EINVAL, "unsupported flags"));
    }

    let policy = access_sched_attr_with(tid, ctx, |attr| Ok(attr.policy()))?;
    let attr: LinuxSchedAttr = policy.try_into()?;
    attr.write_to_user(addr, user_size, ctx)?;

    Ok(SyscallReturn::Return(0))
}

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

    let space = CurrentUserSpace::new(ctx.task);
    let attr: LinuxSchedAttr = space.read_val(addr)?;
    let policy = SchedPolicy::try_from(attr)?;
    access_sched_attr_with(tid, ctx, |attr| {
        attr.set_policy(policy);
        Ok(())
    })?;

    Ok(SyscallReturn::Return(0))
}

pub fn sys_sched_get_priority_min(policy: u32, _: &Context) -> Result<SyscallReturn> {
    let range = SCHED_PRIORITY_RANGE
        .get(policy as usize)
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "invalid scheduling policy"))?;
    Ok(SyscallReturn::Return(*range.start() as isize))
}

pub fn sys_sched_get_priority_max(policy: u32, _: &Context) -> Result<SyscallReturn> {
    let range = SCHED_PRIORITY_RANGE
        .get(policy as usize)
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "invalid scheduling policy"))?;
    Ok(SyscallReturn::Return(*range.end() as isize))
}

pub fn sys_sched_getparam(tid: Tid, addr: Vaddr, ctx: &Context) -> Result<SyscallReturn> {
    let policy = access_sched_attr_with(tid, ctx, |attr| Ok(attr.policy()))?;
    let rt_prio = i32::from(match policy {
        SchedPolicy::RealTime { rt_prio, .. } => rt_prio.get(),
        _ => 0,
    });

    let space = CurrentUserSpace::new(ctx.task);
    space.write_val(addr, &rt_prio)?;

    Ok(SyscallReturn::Return(0))
}

pub fn sys_sched_setparam(tid: Tid, addr: Vaddr, ctx: &Context) -> Result<SyscallReturn> {
    let space = CurrentUserSpace::new(ctx.task);
    let prio: i32 = space.read_val(addr)?;

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

pub fn sys_sched_getscheduler(tid: Tid, ctx: &Context) -> Result<SyscallReturn> {
    let policy = access_sched_attr_with(tid, ctx, |attr| Ok(attr.policy()))?;
    let policy = LinuxSchedAttr::try_from(policy)?.sched_policy;
    Ok(SyscallReturn::Return(policy as isize))
}

pub fn sys_sched_setscheduler(
    tid: Tid,
    policy: i32,
    addr: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let space = CurrentUserSpace::new(&ctx.task);
    let prio = space.read_val(addr)?;

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
