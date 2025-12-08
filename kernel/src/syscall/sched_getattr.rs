// SPDX-License-Identifier: MPL-2.0

use ostd::{const_assert, mm::VmIo};

use super::{
    SyscallReturn,
    sched_get_priority_max::{SCHED_PRIORITY_RANGE, rt_to_static, static_to_rt},
};
use crate::{
    prelude::*,
    process::posix_thread::thread_table,
    sched::{Nice, RealTimePolicy, SchedAttr, SchedPolicy},
    thread::Tid,
    util::CopyCompat,
};

pub(super) const SCHED_NORMAL: u32 = 0;
pub(super) const SCHED_FIFO: u32 = 1;
pub(super) const SCHED_RR: u32 = 2;
// pub(super) const SCHED_BATCH: u32 = 3; // Not supported.
// SCHED_ISO: Reserved but not implemented yet on Linux.
pub(super) const SCHED_IDLE: u32 = 5;
// pub(super) const SCHED_DEADLINE: u32 = 6; // Not supported.
// pub(super) const SCHED_EXT: u32 = 7; // Not supported.

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

// Reference: <https://elixir.bootlin.com/linux/v6.17.7/source/include/uapi/linux/sched/types.h#L7>
const SCHED_ATTR_SIZE_VER0: u32 = 48;
// Reference: <https://elixir.bootlin.com/linux/v6.17.7/source/include/uapi/linux/sched/types.h#L8>
const SCHED_ATTR_SIZE_VER1: u32 = 56;

const_assert!(size_of::<LinuxSchedAttr>() == SCHED_ATTR_SIZE_VER1 as usize);

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

            SchedPolicy::Idle => return_errno_with_message!(
                Errno::EACCES,
                "scheduling attributes for idle tasks are not accessible"
            ),
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
                return_errno_with_message!(Errno::EINVAL, "invalid scheduling priority")
            }

            SCHED_NORMAL => SchedPolicy::Fair(Nice::new(
                i8::try_from(value.sched_nice)
                    .ok()
                    .and_then(|n| n.try_into().ok())
                    .ok_or_else(|| Error::with_message(Errno::EINVAL, "invalid nice number"))?,
            )),

            // The SCHED_IDLE policy is mapped to the highest nice value of
            // `SchedPolicy::Fair` instead of `SchedPolicy::Idle`. Tasks of the
            // latter policy are invisible to the user API.
            SCHED_IDLE => SchedPolicy::Fair(Nice::MAX),

            _ => return_errno_with_message!(Errno::EINVAL, "invalid scheduling policy"),
        })
    }
}

pub(super) fn read_linux_sched_attr_from_user(
    addr: Vaddr,
    ctx: &Context,
) -> Result<LinuxSchedAttr> {
    // The code below is written according to the Linux implementation.
    // Reference: <https://elixir.bootlin.com/linux/v6.17.7/source/kernel/sched/syscalls.c#L889>

    let user_space = ctx.user_space();

    let raw_size = user_space.read_val::<u32>(addr)?;
    let user_size = if raw_size == 0 {
        SCHED_ATTR_SIZE_VER0
    } else {
        raw_size
    };
    if user_size < SCHED_ATTR_SIZE_VER0 || user_size > PAGE_SIZE as u32 {
        let _ = user_space.write_val(addr, &(size_of::<LinuxSchedAttr>() as u32));
        return_errno_with_message!(Errno::E2BIG, "invalid scheduling attribute size");
    }

    let mut attr = user_space
        .read_val_compat::<LinuxSchedAttr>(addr, user_size as usize)
        .inspect_err(|err| {
            if err.error() == Errno::E2BIG {
                let _ = user_space.write_val(addr, &(size_of::<LinuxSchedAttr>() as u32));
            }
        })?;
    // If `attr.size` is modified concurrently, we should use the original size.
    attr.size = user_size;

    // TODO: Check whether `sched_flags` is valid.

    Ok(attr)
}

pub(super) fn write_linux_sched_attr_to_user(
    mut attr: LinuxSchedAttr,
    addr: Vaddr,
    user_size: u32,
    ctx: &Context,
) -> Result<()> {
    if user_size < SCHED_ATTR_SIZE_VER0 || user_size > PAGE_SIZE as u32 {
        return_errno_with_message!(Errno::EINVAL, "invalid scheduling attribute size");
    }

    attr.size = (size_of::<LinuxSchedAttr>() as u32).min(user_size);

    let range = &SCHED_PRIORITY_RANGE[attr.sched_policy as usize];
    attr.sched_util_min = *range.start();
    attr.sched_util_max = *range.end();

    ctx.user_space()
        .write_val_compat(addr, user_size as usize, &attr)?
        .ignore_trailing();

    Ok(())
}

pub(super) fn access_sched_attr_with<T>(
    tid: Tid,
    ctx: &Context,
    f: impl FnOnce(&SchedAttr) -> Result<T>,
) -> Result<T> {
    if tid.cast_signed() < 0 {
        return_errno_with_message!(Errno::EINVAL, "all negative TIDs are not valid");
    }

    if tid == 0 {
        return f(ctx.thread.sched_attr());
    }

    let Some(thread) = thread_table::get_thread(tid) else {
        return_errno_with_message!(Errno::ESRCH, "the target thread does not exist");
    };
    f(thread.sched_attr())
}

pub fn sys_sched_getattr(
    tid: Tid,
    addr: Vaddr,
    user_size: u32,
    flags: u32,
    ctx: &Context,
) -> Result<SyscallReturn> {
    if addr == 0 {
        return_errno_with_message!(Errno::EINVAL, "invalid user space address");
    }
    if flags != 0 {
        // Linux also has no support for any flags yet.
        return_errno_with_message!(Errno::EINVAL, "invalid flags");
    }

    let policy = access_sched_attr_with(tid, ctx, |attr| Ok(attr.policy()))?;
    let attr: LinuxSchedAttr = policy
        .try_into()
        .expect("all user-visible scheduling attributes should be valid");
    write_linux_sched_attr_to_user(attr, addr, user_size, ctx)?;

    Ok(SyscallReturn::Return(0))
}
