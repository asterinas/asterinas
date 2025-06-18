// SPDX-License-Identifier: MPL-2.0

use core::mem;

use super::{
    sched_get_priority_max::{rt_to_static, static_to_rt, SCHED_PRIORITY_RANGE},
    SyscallReturn,
};
use crate::{
    prelude::*,
    process::posix_thread::thread_table,
    sched::{Nice, RealTimePolicy, SchedAttr, SchedPolicy},
    thread::Tid,
};

pub(super) const SCHED_NORMAL: u32 = 0;
pub(super) const SCHED_FIFO: u32 = 1;
pub(super) const SCHED_RR: u32 = 2;
// pub(super) const SCHED_BATCH: u32 = 3; // not supported (never).
// SCHED_ISO: reserved but not implemented yet on Linux.
pub(super) const SCHED_IDLE: u32 = 5;
// pub(super) const SCHED_DEADLINE: u32 = 6; // not supported yet.
// pub(super) const SCHED_EXT: u32 = 7; // not supported (never).

#[derive(Default, Debug, Pod, Clone, Copy)]
#[repr(C)]
pub(super) struct LinuxSchedAttr {
    // Size of this structure
    pub(super) size: u32,

    // Policy (SCHED_*)
    pub(super) sched_policy: u32,
    // Flags
    pub(super) sched_flags: u64,

    // Nice value (SCHED_NORMAL, SCHED_BATCH)
    pub(super) sched_nice: i32,

    // Static priority (SCHED_FIFO, SCHED_RR)
    pub(super) sched_priority: u32,

    // For SCHED_DEADLINE
    pub(super) sched_runtime: u64,
    pub(super) sched_deadline: u64,
    pub(super) sched_period: u64,

    // Utilization hints
    pub(super) sched_util_min: u32,
    pub(super) sched_util_max: u32,
}

impl TryFrom<SchedPolicy> for LinuxSchedAttr {
    type Error = Error;

    fn try_from(value: SchedPolicy) -> Result<Self> {
        Ok(match value {
            SchedPolicy::Stop => LinuxSchedAttr {
                sched_policy: SCHED_FIFO,
                sched_priority: 99, // Linux uses 99 as the default priority for STOP tasks.
                ..Default::default()
            },

            SchedPolicy::RealTime { rt_prio, rt_policy } => LinuxSchedAttr {
                sched_policy: match rt_policy {
                    RealTimePolicy::Fifo => SCHED_FIFO,
                    RealTimePolicy::RoundRobin { .. } => SCHED_RR,
                },
                sched_priority: rt_to_static(rt_prio),
                ..Default::default()
            },

            // The SCHED_IDLE policy is mapped to the highest nice value of
            // `SchedPolicy::Fair` instead of `SchedPolicy::Idle`. Tasks of the
            // latter policy are invisible to the user API.
            SchedPolicy::Fair(Nice::MAX) => LinuxSchedAttr {
                sched_policy: SCHED_IDLE,
                ..Default::default()
            },

            SchedPolicy::Fair(nice) => LinuxSchedAttr {
                sched_policy: SCHED_NORMAL,
                sched_nice: nice.value().get().into(),
                ..Default::default()
            },

            SchedPolicy::Idle => {
                return Err(Error::with_message(
                    Errno::EACCES,
                    "attr for idle tasks are not accessible",
                ))
            }
        })
    }
}

impl TryFrom<LinuxSchedAttr> for SchedPolicy {
    type Error = Error;

    fn try_from(value: LinuxSchedAttr) -> Result<Self> {
        Ok(match value.sched_policy {
            SCHED_FIFO | SCHED_RR => SchedPolicy::RealTime {
                rt_prio: static_to_rt(value.sched_priority)?,
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

            // The SCHED_IDLE policy is mapped to the highest nice value of
            // `SchedPolicy::Fair` instead of `SchedPolicy::Idle`. Tasks of the
            // latter policy are invisible to the user API.
            SCHED_IDLE => SchedPolicy::Fair(Nice::MAX),

            _ => {
                return Err(Error::with_message(
                    Errno::EINVAL,
                    "Invalid scheduling policy",
                ))
            }
        })
    }
}

pub(super) fn read_linux_sched_attr_from_user(
    addr: Vaddr,
    ctx: &Context,
) -> Result<LinuxSchedAttr> {
    let type_size = mem::size_of::<LinuxSchedAttr>();

    let space = ctx.user_space();

    let mut attr = LinuxSchedAttr::default();

    space.read_bytes(
        addr,
        &mut VmWriter::from(&mut attr.as_bytes_mut()[..mem::size_of::<u32>()]),
    )?;

    let size = type_size.min(attr.size as usize);
    space.read_bytes(addr, &mut VmWriter::from(&mut attr.as_bytes_mut()[..size]))?;

    if let Some(additional_size) = attr.size.checked_sub(type_size as u32) {
        let mut buf = vec![0; additional_size as usize];
        space.read_bytes(addr + type_size, &mut VmWriter::from(&mut *buf))?;

        if buf.iter().any(|&b| b != 0) {
            return Err(Error::with_message(Errno::E2BIG, "too big sched_attr"));
        }
    }

    Ok(attr)
}

pub(super) fn write_linux_sched_attr_to_user(
    mut attr: LinuxSchedAttr,
    addr: Vaddr,
    user_size: u32,
    ctx: &Context,
) -> Result<()> {
    let space = ctx.user_space();

    attr.size = (mem::size_of::<LinuxSchedAttr>() as u32).min(user_size);

    let range = SCHED_PRIORITY_RANGE
        .get(attr.sched_policy as usize)
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "invalid scheduling policy"))?;
    attr.sched_util_min = *range.start();
    attr.sched_util_max = *range.end();

    space.write_bytes(
        addr,
        &mut VmReader::from(&attr.as_bytes()[..attr.size as usize]),
    )?;
    Ok(())
}

pub(super) fn access_sched_attr_with<T>(
    tid: Tid,
    ctx: &Context,
    f: impl FnOnce(&SchedAttr) -> Result<T>,
) -> Result<T> {
    match tid {
        0 => f(ctx.thread.sched_attr()),
        _ if tid > (i32::MAX as u32) => Err(Error::with_message(Errno::EINVAL, "invalid tid")),
        _ => f(thread_table::get_thread(tid)
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
    write_linux_sched_attr_to_user(attr, addr, user_size, ctx)
        .map_err(|_| Error::new(Errno::EINVAL))?;

    Ok(SyscallReturn::Return(0))
}
