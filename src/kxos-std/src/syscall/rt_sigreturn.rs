use crate::{memory::read_val_from_user, prelude::*, process::signal::c_types::ucontext_t};
use kxos_frame::cpu::CpuContext;

use super::SyscallReturn;

pub fn sys_rt_sigreturn(context: &mut CpuContext) -> Result<SyscallReturn> {
    let current = current!();
    let sig_context = current.sig_context().lock().pop_back().unwrap();
    let ucontext = read_val_from_user::<ucontext_t>(sig_context)?;
    context.gp_regs = ucontext.uc_mcontext.inner.gp_regs;
    // unblock sig mask
    let sig_mask = ucontext.uc_sigmask;
    current.sig_mask().lock().unblock(sig_mask);
    Ok(SyscallReturn::NoReturn)
}
