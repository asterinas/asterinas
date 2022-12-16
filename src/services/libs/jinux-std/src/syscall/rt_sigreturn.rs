use crate::{
    log_syscall_entry, prelude::*, process::signal::c_types::ucontext_t, util::read_val_from_user,
};
use jinux_frame::cpu::CpuContext;

use super::{SyscallReturn, SYS_RT_SIGRETRUN};

pub fn sys_rt_sigreturn(context: &mut CpuContext) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_RT_SIGRETRUN);
    let current = current!();
    let sig_context = current.sig_context().lock().pop_back().unwrap();
    let ucontext = read_val_from_user::<ucontext_t>(sig_context)?;
    context.gp_regs = ucontext.uc_mcontext.inner.gp_regs;
    // unblock sig mask
    let sig_mask = ucontext.uc_sigmask;
    current.sig_mask().lock().unblock(sig_mask);
    Ok(SyscallReturn::NoReturn)
}
