// SPDX-License-Identifier: MPL-2.0

use alloc::{sync::Arc, vec::Vec};
use core::sync::atomic::Ordering;

use super::SyscallReturn;
use crate::{
    prelude::*,
    process::{posix_thread::AsPosixThread, ProcessGroup},
    thread::Thread,
};

#[expect(dead_code)]
pub(super) enum IoPrioWho {
    Thread(Arc<Thread>),
    ProcessGroup(Arc<ProcessGroup>),
    /// The threads belong to the same user
    User(Vec<Arc<Thread>>),
}

impl IoPrioWho {
    const IOPRIO_WHO_PROCESS: u32 = 1;
    const IOPRIO_WHO_PGRP: u32 = 2;
    const IOPRIO_WHO_USER: u32 = 3;

    pub(super) fn from_which_and_who(which: u32, who: u32, ctx: &Context) -> Result<Self> {
        match which {
            Self::IOPRIO_WHO_PROCESS => {
                // In Linux, IOPRIO_WHO_PROCESS identifies a single thread by its TID
                let target_tid = if who == 0 {
                    ctx.posix_thread.tid()
                } else {
                    who
                };

                let thread = crate::process::posix_thread::thread_table::get_thread(target_tid)
                    .ok_or_else(|| Error::new(Errno::ESRCH))?;
                Ok(Self::Thread(thread))
            }
            Self::IOPRIO_WHO_PGRP => {
                // TODO: Implement process group support
                return_errno!(Errno::EINVAL);
            }
            Self::IOPRIO_WHO_USER => {
                // TODO: Implement user support
                return_errno!(Errno::EINVAL);
            }
            _ => return_errno!(Errno::EINVAL),
        }
    }
}

pub fn sys_ioprio_get(which: u32, who: u32, ctx: &Context) -> Result<SyscallReturn> {
    debug!("which = {}, who = {}", which, who);

    let ioprio_who = IoPrioWho::from_which_and_who(which, who, ctx)?;

    match ioprio_who {
        IoPrioWho::Thread(thread) => {
            let prio = thread
                .as_posix_thread()
                .unwrap()
                .io_priority()
                .load(Ordering::Relaxed);
            Ok(SyscallReturn::Return(prio as _))
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
