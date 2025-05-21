// SPDX-License-Identifier: MPL-2.0

use ostd::{cpu::PinCurrentCpu, task::disable_preempt};

use super::SyscallReturn;
use crate::prelude::*;

pub fn sys_getcpu(cpu: Vaddr, node: Vaddr, _tcache: Vaddr, ctx: &Context) -> Result<SyscallReturn> {
    // The third argument tcache is unused after Linux 2.6.24 so we ignore it
    let preempt_guard = disable_preempt();
    let cpuid = preempt_guard.current_cpu();
    drop(preempt_guard);
    debug!(
        "getcpu: cpuid = {}, total_cpus = {}",
        cpuid.as_usize(),
        ostd::cpu::num_cpus()
    );
    // Since cpu and node can be NULL, we need to check them before writing
    if cpu != 0 {
        ctx.user_space()
            .write_val::<usize>(cpu, &cpuid.as_usize())?;
    }
    if node != 0 {
        ctx.user_space().write_val::<usize>(node, &0)?; // TODO: NUMA is not supported
    }
    Ok(SyscallReturn::Return(0))
}
