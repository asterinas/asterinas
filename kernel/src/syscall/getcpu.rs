// SPDX-License-Identifier: MPL-2.0

use ostd::{cpu::PinCurrentCpu, task::disable_preempt};

use super::SyscallReturn;
use crate::prelude::*;

pub fn sys_getcpu(cpu: Vaddr, node: Vaddr, ctx: &Context) -> Result<SyscallReturn> {
    let preempt_guard = disable_preempt();
    let cpuid = preempt_guard.current_cpu();
    ctx.user_space()
        .write_val::<usize>(cpu, &cpuid.as_usize())?;
    ctx.user_space().write_val::<usize>(node, &0)?; // TODO: NUMA is not supported
    Ok(SyscallReturn::Return(0))
}
