use crate::prelude::*;
use kxos_frame::cpu::CpuContext;

use super::SyscallReturn;

pub fn sys_rt_sigreturn(context: &mut CpuContext) -> Result<SyscallReturn> {
    let current = current!();
    let sig_context = current.sig_context().lock().as_ref().unwrap().clone();
    *context = *sig_context.cpu_context();
    // unblock sig mask
    let sig_mask = sig_context.sig_mask();
    current.sig_mask().lock().unblock(sig_mask.as_u64());
    Ok(SyscallReturn::NoReturn)
}
