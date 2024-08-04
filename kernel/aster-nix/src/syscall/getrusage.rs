// SPDX-License-Identifier: MPL-2.0

use int_to_c_enum::TryFromInt;

use super::{CurrentInfo, SyscallReturn};
use crate::{prelude::*, time::timeval_t};

#[derive(Debug, Copy, Clone, TryFromInt, PartialEq)]
#[repr(i32)]
#[allow(non_camel_case_types)]
enum RusageTarget {
    ForSelf = 0,
    Children = -1,
    Both = -2,
    Thread = 1,
}

pub fn sys_getrusage(
    target: i32,
    rusage_addr: Vaddr,
    current: CurrentInfo,
) -> Result<SyscallReturn> {
    let CurrentInfo {
        process,
        posix_thread,
        thread: _,
        task: _,
    } = current;

    let rusage_target = RusageTarget::try_from(target)?;

    debug!(
        "target = {:?}, rusage_addr = {}",
        rusage_target, rusage_addr,
    );

    if rusage_addr != 0 {
        let rusage = match rusage_target {
            RusageTarget::ForSelf => rusage_t {
                ru_utime: process.prof_clock().user_clock().read_time().into(),
                ru_stime: process.prof_clock().kernel_clock().read_time().into(),
                ..Default::default()
            },
            RusageTarget::Thread => rusage_t {
                ru_utime: posix_thread.prof_clock().user_clock().read_time().into(),
                ru_stime: posix_thread.prof_clock().kernel_clock().read_time().into(),
                ..Default::default()
            },
            // To support `Children` and `Both` we need to implement the functionality to
            // accumulate the resources of a child process back to the parent process
            // upon the child's termination.
            _ => {
                return_errno_with_message!(Errno::EINVAL, "the target type is not supported")
            }
        };

        CurrentUserSpace::get().write_val(rusage_addr, &rusage)?;
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
    /// voluntary context switches
    pub ru_nvcsw: u64,
    /// involuntary
    pub ru_nivcsw: u64,
}
