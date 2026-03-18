// SPDX-License-Identifier: MPL-2.0

use ostd::{
    arch::cpu::context::{FpuContext, UserContext},
    mm::VmIo,
    user::UserContextApi,
};

use super::SyscallReturn;
use crate::{
    prelude::*,
    process::{posix_thread::ContextPthreadAdminApi, signal::c_types::ucontext_t},
    syscall::sigaltstack::set_new_stack,
};

pub fn sys_rt_sigreturn(ctx: &Context, user_ctx: &mut UserContext) -> Result<SyscallReturn> {
    let sig_context_addr = user_ctx.stack_pointer() as Vaddr;
    debug!(
        "sys_rt_sigreturn: sig_context_addr = {:#x}",
        sig_context_addr
    );

    let ucontext = ctx.user_space().read_val::<ucontext_t>(sig_context_addr)?;

    // Restore general-purpose registers.
    ucontext.uc_mcontext.copy_user_regs_to(user_ctx);

    // Restore signal stack settings.
    let stack = ucontext.uc_stack;
    // If the stack setting is invalid, we silently ignore the error,
    // following Linux's behavior.
    // Reference: <https://elixir.bootlin.com/linux/v6.15/source/kernel/signal.c#L4456>.
    let _ = set_new_stack(stack, ctx, user_ctx.stack_pointer());

    // Restore the FPU context.
    cfg_if::cfg_if! {
        if #[cfg(target_arch = "x86_64")] {
            let fpu_context_addr = ucontext.uc_mcontext.fpu_context_addr();
        } else if #[cfg(any(target_arch = "riscv64", target_arch = "loongarch64"))] {
            // In RISC-V/LoongArch64, the FPU context is placed directly after `ucontext_t` on the
            // signal stack.
            let fpu_context_addr = sig_context_addr + size_of::<ucontext_t>();
        } else {
            compile_error!("unsupported target");
        }
    }
    let mut fpu_context = FpuContext::new();
    ctx.user_space()
        .read_bytes(fpu_context_addr, fpu_context.as_bytes_mut())?;
    ctx.thread_local.fpu().set_context(fpu_context);

    // Restore the signal mask.
    let sig_mask = ucontext.uc_sigmask;
    ctx.set_sig_mask(sig_mask.into());

    Ok(SyscallReturn::NoReturn)
}
