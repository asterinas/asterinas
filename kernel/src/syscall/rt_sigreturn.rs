// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::Ordering;

use ostd::{
    cpu::context::{FpuContext, UserContext},
    user::UserContextApi,
};

use super::SyscallReturn;
use crate::{
    prelude::*, process::signal::c_types::ucontext_t, syscall::sigaltstack::set_new_stack,
};

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

    // Set previous ucontext address
    if ucontext.uc_link == 0 {
        thread_local.sig_context().set(None);
    } else {
        thread_local.sig_context().set(Some(ucontext.uc_link));
    };
    ucontext.uc_mcontext.copy_user_regs_to(user_ctx);

    // Restore signal stack settings
    let stack = ucontext.uc_stack;
    // If the stack setting is invalid, we silently ignore the error,
    // following Linux's behavior.
    // Reference: <https://elixir.bootlin.com/linux/v6.15/source/kernel/signal.c#L4456>.
    let _ = set_new_stack(stack, ctx, user_ctx.stack_pointer());

    // Restore FPU context on stack
    cfg_if::cfg_if! {
        if #[cfg(target_arch = "x86_64")] {
            let fpu_context_addr = ucontext.uc_mcontext.fpu_context_addr();
        } else if #[cfg(any(target_arch = "riscv64", target_arch = "loongarch64"))] {
            // In RISC-V/LoongArch64, FPU context is placed directly after `ucontext_t` on signal stack.
            let fpu_context_addr = sig_context_addr + size_of::<ucontext_t>();
        } else {
            compile_error!("unsupported target");
        }
    }
    let mut fpu_context = FpuContext::new();
    let mut fpu_context_writer = VmWriter::from(fpu_context.as_bytes_mut());
    ctx.user_space()
        .read_bytes(fpu_context_addr, &mut fpu_context_writer)?;
    ctx.thread_local.fpu().set_context(fpu_context);

    // unblock sig mask
    let sig_mask = ucontext.uc_sigmask;
    let old_mask = posix_thread.sig_mask().load(Ordering::Relaxed);
    posix_thread
        .sig_mask()
        .store(old_mask - sig_mask, Ordering::Relaxed);

    Ok(SyscallReturn::NoReturn)
}
