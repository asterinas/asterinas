use crate::{
    log_syscall_entry,
    prelude::*,
    process::{posix_thread::PosixThreadExt, signal::c_types::ucontext_t},
    util::read_val_from_user,
};
use aster_frame::cpu::UserContext;

use super::{SyscallReturn, SYS_RT_SIGRETRUN};

pub fn sys_rt_sigreturn(context: &mut UserContext) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_RT_SIGRETRUN);
    let current_thread = current_thread!();
    let posix_thread = current_thread.as_posix_thread().unwrap();
    let mut sig_context = posix_thread.sig_context().lock();
    if (*sig_context).is_none() {
        return_errno_with_message!(Errno::EINVAL, "sigretrun should not been called");
    }
    let sig_context_addr = sig_context.unwrap();
    // FIXME: This assertion is not always true, if RESTORER flag is not presented.
    // In this case, we will put restorer code on user stack, then the assertion will fail.
    // However, for most glibc applications, the restorer codes is provided by glibc and RESTORER flag is set.
    debug_assert!(sig_context_addr == context.rsp() as Vaddr);

    let ucontext = read_val_from_user::<ucontext_t>(sig_context_addr)?;

    // If the sig stack is active and used by current handler, decrease handler counter.
    if let Some(sig_stack) = posix_thread.sig_stack().lock().as_mut() {
        let rsp = context.rsp();
        if rsp >= sig_stack.base() && rsp <= sig_stack.base() + sig_stack.size() {
            sig_stack.decrease_handler_counter();
        }
    }

    // Set previous ucontext address
    if ucontext.uc_link == 0 {
        *sig_context = None;
    } else {
        *sig_context = Some(ucontext.uc_link);
    };
    *context.general_regs_mut() = ucontext.uc_mcontext.inner.gp_regs;
    // unblock sig mask
    let sig_mask = ucontext.uc_sigmask;
    posix_thread.sig_mask().lock().unblock(sig_mask);

    Ok(SyscallReturn::NoReturn)
}
