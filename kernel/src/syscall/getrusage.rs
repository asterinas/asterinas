// SPDX-License-Identifier: MPL-2.0

use int_to_c_enum::TryFromInt;
use ostd::mm::VmIo;

use super::SyscallReturn;
use crate::{prelude::*, time::timeval_t};

#[derive(Debug, Copy, Clone, TryFromInt, PartialEq)]
#[repr(i32)]
enum RusageTarget {
    ForSelf = 0,
    Children = -1,
    Thread = 1,
}

pub fn sys_getrusage(target: i32, rusage_addr: Vaddr, ctx: &Context) -> Result<SyscallReturn> {
    let rusage_target = RusageTarget::try_from(target)?;

    debug!(
        "target = {:?}, rusage_addr = {}",
        rusage_target, rusage_addr,
    );

    if rusage_addr != 0 {
        let rusage = match rusage_target {
            RusageTarget::ForSelf => {
                let process = ctx.process.as_ref();
                rusage_t {
                    ru_utime: process.prof_clock().user_clock().read_time().into(),
                    ru_stime: process.prof_clock().kernel_clock().read_time().into(),
                    ..Default::default()
                }
            }
            RusageTarget::Thread => {
                let posix_thread = ctx.posix_thread;
                rusage_t {
                    ru_utime: posix_thread.prof_clock().user_clock().read_time().into(),
                    ru_stime: posix_thread.prof_clock().kernel_clock().read_time().into(),
                    ..Default::default()
                }
            }
            RusageTarget::Children => {
                let (user_time, kernel_time) = ctx.process.reaped_children_stats().lock().get();
                rusage_t {
                    ru_utime: user_time.into(),
                    ru_stime: kernel_time.into(),
                    ..Default::default()
                }
            }
        };

        ctx.user_space().write_val(rusage_addr, &rusage)?;
    }

    Ok(SyscallReturn::Return(0))
}

#[repr(C)]
#[derive(Debug, Default, Copy, Clone, Pod)]
pub struct rusage_t {
    /// user time used
    pub ru_utime: timeval_t,
    /// system time used
    pub ru_stime: timeval_t,
    /// maximum resident set size
    pub ru_maxrss: u64,
    /// integral shared memory size
    pub ru_ixrss: u64,
    /// integral unshared data size
    pub ru_idrss: u64,
    /// integral unshared stack size
    pub ru_isrss: u64,
    /// page reclaims
    pub ru_minflt: u64,
    /// page faults
    pub ru_majflt: u64,
    /// swaps
    pub ru_nswap: u64,
    /// block input operations
    pub ru_inblock: u64,
    /// block output operations
    pub ru_oublock: u64,
    /// messages sent
    pub ru_msgsnd: u64,
    /// messages received
    pub ru_msgrcv: u64,
    /// signals received
    pub ru_nsignals: u64,
    /// voluntary ctx switches
    pub ru_nvcsw: u64,
    /// involuntary
    pub ru_nivcsw: u64,
}
