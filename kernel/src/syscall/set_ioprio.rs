// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::Ordering;

use super::{get_ioprio::IoPrioWho, SyscallReturn};
use crate::{prelude::*, process::posix_thread::AsPosixThread};

pub fn sys_ioprio_set(which: u32, who: u32, ioprio: u32, ctx: &Context) -> Result<SyscallReturn> {
    debug!("which = {}, who = {}, ioprio = {}", which, who, ioprio);

    let ioprio_who = IoPrioWho::from_which_and_who(which, who, ctx)?;

    match ioprio_who {
        IoPrioWho::Thread(thread) => {
            thread
                .as_posix_thread()
                .unwrap()
                .io_priority()
                .store(ioprio, Ordering::Relaxed);
            Ok(SyscallReturn::Return(0))
        }
        IoPrioWho::ProcessGroup(_) => {
            // TODO: Implement process group support
            return_errno!(Errno::EINVAL);
        }
        IoPrioWho::User(_) => {
            // TODO: Implement user support
            return_errno!(Errno::EINVAL);
        }
    }
}
