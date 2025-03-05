// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::Ordering;

use ostd::{cpu::context::UserContext, user::UserContextApi};

use super::SyscallReturn;
use crate::{prelude::*, process::signal::c_types::ucontext_t};

pub fn sys_rt_sigreturn(ctx: &Context, user_ctx: &mut UserContext) -> Result<SyscallReturn> {
    let Context {
        thread_local,
        posix_thread,
        ..
    } = ctx;

    let Some(sig_context_addr) = thread_local.sig_context().get() else {
        return_errno_with_message!(
            Errno::EINVAL,
            "`sigreturn` cannot be called outside the signal context"
        );
    };
    // FIXME: This assertion is not always true, if RESTORER flag is not presented.
    // In this case, we will put restorer code on user stack, then the assertion will fail.
    // However, for most glibc applications, the restorer codes is provided by glibc and RESTORER flag is set.
    debug_assert!(sig_context_addr == user_ctx.stack_pointer() as Vaddr);

    let ucontext = ctx.user_space().read_val::<ucontext_t>(sig_context_addr)?;

    // If the sig stack is active and used by current handler, decrease handler counter.
    if let Some(sig_stack) = &mut *thread_local.sig_stack().borrow_mut() {
        let rsp = user_ctx.stack_pointer();
        if rsp >= sig_stack.base() && rsp <= sig_stack.base() + sig_stack.size() {
            sig_stack.decrease_handler_counter();
        }
    }

    // Set previous ucontext address
    if ucontext.uc_link == 0 {
        thread_local.sig_context().set(None);
    } else {
        thread_local.sig_context().set(Some(ucontext.uc_link));
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
