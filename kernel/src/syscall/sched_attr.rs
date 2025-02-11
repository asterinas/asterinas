// SPDX-License-Identifier: MPL-2.0

use core::mem;

use ostd::task::Task;

use super::SyscallReturn;
use crate::{
    prelude::*,
    process::posix_thread::thread_table,
    sched::{Nice, RealTimePolicy, SchedPolicy},
    thread::Tid,
};

const SCHED_NORMAL: u32 = 0;
const SCHED_FIFO: u32 = 1;
const SCHED_RR: u32 = 2;
// const SCHED_BATCH: u32 = 3; // not supported yet.
// SCHED_ISO: reserved but not implemented yet on Linux.
const SCHED_IDLE: u32 = 5;
// const SCHED_DEADLINE: u32 = 6; // not supported yet.
// const SCHED_EXT: u32 = 7; // not supported yet.

#[derive(Default, Debug, Pod, Clone, Copy)]
#[repr(C)]
struct PosixSchedAttr {
    size: u32,

    sched_policy: u32,
    sched_flags: u64,

    // SCHED_NORMAL, SCHED_BATCH
    sched_nice: i32,

    // SCHED_FIFO, SCHED_RR
    sched_priority: u32,

    // SCHED_DEADLINE
    sched_runtime: u64,
    sched_deadline: u64,
    sched_period: u64,

    // Utilization hints
    sched_util_min: u32,
    sched_util_max: u32,
}

impl TryFrom<SchedPolicy> for PosixSchedAttr {
    type Error = Error;

    fn try_from(value: SchedPolicy) -> Result<Self> {
        Ok(match value {
            SchedPolicy::Stop => return Err(Error::new(Errno::EACCES)),

            SchedPolicy::RealTime { rt_prio, rt_policy } => PosixSchedAttr {
                sched_policy: match rt_policy {
                    RealTimePolicy::Fifo => SCHED_FIFO,
                    RealTimePolicy::RoundRobin { .. } => SCHED_RR,
                },
                sched_priority: u32::from(rt_prio.get()),
                ..Default::default()
            },

            SchedPolicy::Fair(nice) => PosixSchedAttr {
                sched_policy: SCHED_NORMAL,
                sched_nice: i32::from(nice.value().get()),
                ..Default::default()
            },

            SchedPolicy::Idle => PosixSchedAttr {
                sched_policy: SCHED_IDLE,
                ..Default::default()
            },
        })
    }
}

impl TryFrom<PosixSchedAttr> for SchedPolicy {
    type Error = Error;

    fn try_from(value: PosixSchedAttr) -> Result<Self> {
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

impl PosixSchedAttr {
    fn read_from_user(addr: Vaddr) -> Result<Self> {
        let task = Task::current().unwrap();
        let space = CurrentUserSpace::new(&task);

        Ok(space.read_val(addr)?)
    }

    fn write_to_user(mut self, addr: Vaddr, user_size: u32) -> Result<()> {
        const _: () = assert!(mem::size_of::<PosixSchedAttr>() <= u32::MAX as usize);

        let task = Task::current().unwrap();
        let space = CurrentUserSpace::new(&task);

        self.size = (mem::size_of::<PosixSchedAttr>() as u32).min(user_size);
        space.write_bytes(
            addr,
            &mut VmReader::from(&self.as_bytes()[..self.size as usize]),
        )?;
        Ok(())
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
        return Err(Error::with_message(Errno::EINVAL, "unsupported flags"));
    }

    let policy = match tid {
        0 => ctx.thread.sched_attr().policy(),
        _ => thread_table::get_thread(tid)
            .ok_or_else(|| Error::with_message(Errno::ESRCH, "thread does not exist"))?
            .sched_attr()
            .policy(),
    };

    let attr: PosixSchedAttr = policy.try_into()?;
    attr.write_to_user(addr, user_size)?;

    Ok(SyscallReturn::Return(0))
}

pub fn sys_sched_setattr(
    tid: Tid,
    addr: Vaddr,
    flags: u32,
    ctx: &Context,
) -> Result<SyscallReturn> {
    if flags != 0 {
        return Err(Error::with_message(Errno::EINVAL, "unsupported flags"));
    }

    let attr = PosixSchedAttr::read_from_user(addr)?;
    let policy = SchedPolicy::try_from(attr)?;

    match tid {
        0 => ctx.thread.sched_attr().set_policy(policy),
        _ => thread_table::get_thread(tid)
            .ok_or_else(|| Error::with_message(Errno::ESRCH, "thread does not exist"))?
            .sched_attr()
            .set_policy(policy),
    }

    Ok(SyscallReturn::Return(0))
}

pub fn sys_sched_getparam(tid: Tid, addr: Vaddr, ctx: &Context) -> Result<SyscallReturn> {
    let policy = match tid {
        0 => ctx.thread.sched_attr().policy(),
        _ => thread_table::get_thread(tid)
            .ok_or_else(|| Error::with_message(Errno::ESRCH, "thread does not exist"))?
            .sched_attr()
            .policy(),
    };

    let rt_prio = i32::from(match policy {
        SchedPolicy::RealTime { rt_prio, .. } => rt_prio.get(),
        _ => 0,
    });

    let task = Task::current().unwrap();
    let space = CurrentUserSpace::new(&task);
    space.write_val(addr, &rt_prio)?;

    Ok(SyscallReturn::Return(0))
}

pub fn sys_sched_setparam(tid: Tid, addr: Vaddr, ctx: &Context) -> Result<SyscallReturn> {
    let task = Task::current().unwrap();
    let space = CurrentUserSpace::new(&task);
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

    match tid {
        0 => ctx.thread.sched_attr().update_policy(update)?,
        _ => thread_table::get_thread(tid)
            .ok_or_else(|| Error::with_message(Errno::ESRCH, "thread does not exist"))?
            .sched_attr()
            .update_policy(update)?,
    }

    Ok(SyscallReturn::Return(0))
}

pub fn sys_sched_getscheduler(tid: Tid, ctx: &Context) -> Result<SyscallReturn> {
    let policy = match tid {
        0 => ctx.thread.sched_attr().policy(),
        _ => thread_table::get_thread(tid)
            .ok_or_else(|| Error::with_message(Errno::ESRCH, "thread does not exist"))?
            .sched_attr()
            .policy(),
    };

    let policy = PosixSchedAttr::try_from(policy)?.sched_policy;
    Ok(SyscallReturn::Return(policy as isize))
}

pub fn sys_sched_setscheduler(
    tid: Tid,
    policy: i32,
    addr: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let task = Task::current().unwrap();
    let space = CurrentUserSpace::new(&task);
    let prio = space.read_val(addr)?;

    let attr = PosixSchedAttr {
        sched_policy: policy as u32,
        sched_priority: prio,
        ..Default::default()
    };

    let policy = attr.try_into()?;

    match tid {
        0 => ctx.thread.sched_attr().set_policy(policy),
        _ => thread_table::get_thread(tid)
            .ok_or_else(|| Error::with_message(Errno::ESRCH, "thread does not exist"))?
            .sched_attr()
            .set_policy(policy),
    }

    Ok(SyscallReturn::Return(0))
}
