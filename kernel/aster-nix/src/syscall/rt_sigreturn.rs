// SPDX-License-Identifier: MPL-2.0

use ostd::{cpu::UserContext, user::UserContextApi};

use super::SyscallReturn;
use crate::{
    prelude::*,
    process::{posix_thread::PosixThreadExt, signal::c_types::ucontext_t},
    util::read_val_from_user,
};

pub fn sys_rt_sigreturn(context: &mut UserContext) -> Result<SyscallReturn> {
    let current_thread = current_thread!();
    let posix_thread = current_thread.as_posix_thread().unwrap();
    let mut sig_context = posix_thread.sig_context().lock();
    if (*sig_context).is_none() {
        return_errno_with_message!(Errno::EINVAL, "sigreturn should not been called");
    }
    let sig_context_addr = sig_context.unwrap();
    // FIXME: This assertion is not always true, if RESTORER flag is not presented.
    // In this case, we will put restorer code on user stack, then the assertion will fail.
    // However, for most glibc applications, the restorer codes is provided by glibc and RESTORER flag is set.
    debug_assert!(sig_context_addr == context.stack_pointer() as Vaddr);

    let ucontext = read_val_from_user::<ucontext_t>(sig_context_addr)?;

    // If the sig stack is active and used by current handler, decrease handler counter.
    if let Some(sig_stack) = posix_thread.sig_stack().lock().as_mut() {
        let rsp = context.stack_pointer();
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
    ucontext
        .uc_mcontext
        .inner
        .gp_regs
        .copy_to_raw(context.general_regs_mut());
    // unblock sig mask
    let sig_mask = ucontext.uc_sigmask;
    posix_thread.sig_mask().lock().unblock(sig_mask);

    Ok(SyscallReturn::NoReturn)
}
