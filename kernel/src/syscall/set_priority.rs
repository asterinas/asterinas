// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::Ordering;

use super::SyscallReturn;
use crate::{
    prelude::*,
    process::ResourceType::RLIMIT_NICE,
    sched::Nice,
    syscall::get_priority::{get_processes, PriorityTarget},
};

pub fn sys_set_priority(which: i32, who: u32, prio: i32, ctx: &Context) -> Result<SyscallReturn> {
    let prio_target = PriorityTarget::new(which, who, ctx)?;
    let new_nice: Nice = {
        let nice_raw = prio.clamp(
            Nice::MIN.value().get() as i32,
            Nice::MAX.value().get() as i32,
        ) as i8;
        nice_raw.try_into().unwrap()
    };

    debug!(
        "set_priority prio_target: {:?}, new_nice: {:?}",
        prio_target, new_nice
    );

    let processes = get_processes(prio_target)?;
    for process in processes.iter() {
        let rlimit = process.resource_limits();
        let limit = (rlimit.get_rlimit(RLIMIT_NICE).get_cur() as i8)
            .try_into()
            .map_err(|msg| Error::with_message(Errno::EINVAL, msg))?;

        if new_nice < limit {
            return_errno!(Errno::EACCES);
        }
        process.nice().store(new_nice, Ordering::Relaxed);
    }

    Ok(SyscallReturn::Return(0))
}
