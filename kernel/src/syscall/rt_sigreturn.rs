// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::Ordering;

use ostd::{cpu::UserContext, user::UserContextApi};

use super::SyscallReturn;
use crate::{prelude::*, process::signal::c_types::ucontext_t};

pub fn sys_rt_sigreturn(ctx: &Context, user_ctx: &mut UserContext) -> Result<SyscallReturn> {
    let Context {
        process: _,
        posix_thread,
        thread: _,
        task: _,
    } = ctx;
    let mut sig_context = posix_thread.sig_context().lock();
    if (*sig_context).is_none() {
        return_errno_with_message!(Errno::EINVAL, "sigreturn should not been called");
    }
    let sig_context_addr = sig_context.unwrap();
    // FIXME: This assertion is not always true, if RESTORER flag is not presented.
    // In this case, we will put restorer code on user stack, then the assertion will fail.
    // However, for most glibc applications, the restorer codes is provided by glibc and RESTORER flag is set.
    debug_assert!(sig_context_addr == user_ctx.stack_pointer() as Vaddr);

    let ucontext = ctx.user_space().read_val::<ucontext_t>(sig_context_addr)?;

    // If the sig stack is active and used by current handler, decrease handler counter.
    if let Some(sig_stack) = posix_thread.sig_stack().lock().as_mut() {
        let rsp = user_ctx.stack_pointer();
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
        .copy_to_raw(user_ctx.general_regs_mut());
    // unblock sig mask
    let sig_mask = ucontext.uc_sigmask;
    let old_mask = posix_thread.sig_mask().load(Ordering::Relaxed);
    posix_thread
        .sig_mask()
        .store(old_mask - sig_mask, Ordering::Relaxed);

    Ok(SyscallReturn::NoReturn)
}
