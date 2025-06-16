// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::Ordering;

use super::SyscallReturn;
use crate::{
    prelude::*,
    process::{process_table, Pid},
};

const IOPRIO_WHO_PROCESS: u32 = 1;

pub fn sys_get_ioprio(which: u32, who: u32, ioprio: u32, ctx: &Context) -> Result<SyscallReturn> {
    debug!("which = {}, who = {}, ioprio = {}", which, who, ioprio);

    // Now we only support IOPRIO_WHO_PROCESS
    if which != IOPRIO_WHO_PROCESS {
        return_errno!(Errno::EINVAL);
    }

    let target_pid = if who == 0 {
        ctx.process.pid()
    } else {
        who as Pid
    };

    let process = process_table::get_process(target_pid).ok_or(Error::new(Errno::ESRCH))?;

    let prio = process.io_priority().load(Ordering::Relaxed);

    Ok(SyscallReturn::Return(prio.try_into().unwrap()))
}
