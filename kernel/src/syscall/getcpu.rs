// SPDX-License-Identifier: MPL-2.0

use ostd::cpu::CpuId;

use super::SyscallReturn;
use crate::prelude::*;

pub fn sys_getcpu(cpu: Vaddr, node: Vaddr, _tcache: Vaddr, ctx: &Context) -> Result<SyscallReturn> {
    // The third argument `tcache` is unused since Linux 2.6.24, so we ignore it.

    // The system call itself is inherently racy, so using `current_racy` here should be fine.
    let current_cpu = CpuId::current_racy().as_usize() as u32;
    // TODO: Support NUMA.
    let current_node = 0u32;

    debug!(
        "[sys_getcpu]: cpu = {} (total {}), node = {}",
        current_cpu,
        ostd::cpu::num_cpus(),
        current_node,
    );

    // `cpu` and `node` can be NULL, so we need to check before writing.
    if cpu != 0 {
        ctx.user_space().write_val(cpu, &current_cpu)?;
    }
    if node != 0 {
        ctx.user_space().write_val(node, &current_node)?;
    }

    Ok(SyscallReturn::Return(0))
}
