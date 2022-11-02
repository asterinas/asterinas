use crate::prelude::*;

use crate::{process::Process, syscall::SYS_SCHED_YIELD};

pub fn sys_sched_yield() -> Result<isize> {
    debug!("[syscall][id={}][SYS_SCHED_YIELD]", SYS_SCHED_YIELD);
    Process::yield_now();
    Ok(0)
}
