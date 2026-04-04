// SPDX-License-Identifier: MPL-2.0

pub mod c_types;
pub mod constants;
mod pause;
mod pending;
mod poll;
pub mod sig_action;
pub mod sig_disposition;
pub mod sig_mask;
pub mod sig_num;
pub mod sig_queues;
mod sig_stack;
pub mod signals;

use align_ext::AlignExt;
use c_types::{siginfo_t, ucontext_t};
use constants::SIGSEGV;
use ostd::{
    arch::cpu::context::{FpuContext, UserContext},
    mm::VmIo,
    user::UserContextApi,
};
pub use pause::{Pause, PauseReason, with_sigmask_changed};
pub use pending::HandlePendingSignal;
pub use poll::{PollAdaptor, PollHandle, Pollable, Pollee, Poller};
use sig_action::{SigAction, SigActionFlags, SigDefaultAction};
use sig_mask::SigMask;
use sig_num::SigNum;
pub use sig_stack::{SigStack, SigStackFlags, SigStackStatus};

use super::posix_thread::ThreadLocal;
use crate::{
    cpu::LinuxAbi,
    prelude::*,
    process::{
        TermStatus,
        posix_thread::{ContextPthreadAdminApi, do_exit_group},
        signal::{c_types::stack_t, signals::Signal},
    },
};

pub trait SignalContext {
    /// Sets signal handler arguments.
    fn set_arguments(&mut self, sig_num: SigNum, siginfo_addr: usize, ucontext_addr: usize);
}

/// Handles a pending signal for the current process.
pub fn handle_pending_signal(
    user_ctx: &mut UserContext,
    ctx: &Context,
    pre_syscall_ret: Option<usize>,
) {
    let syscall_restart = if let Some(pre_syscall_ret) = pre_syscall_ret
        && user_ctx.syscall_ret() == -(Errno::ERESTARTSYS as i32) as usize
    {
        // We should never return `ERESTARTSYS` to the userspace.
        user_ctx.set_syscall_ret(-(Errno::EINTR as i32) as usize);
        Some(pre_syscall_ret)
    } else {
        None
    };

    let mut restore_sig_mask = ctx
        .thread_local
        .sig_mask_saved()
        .take()
        .map(|mask| RestoreSigMaskGuard { ctx, mask });

    let (signal, sig_action) = if let Some(dequeued_signal) = dequeue_pending_signal(ctx) {
        dequeued_signal
    } else {
        // Fast path: There is no signal mask to restore.
        if restore_sig_mask.is_none() {
            return;
        }
        // Restore the signal mask first.
        let _ = restore_sig_mask.take();

        // Try again with the new signal mask.
        if let Some(dequeued_signal) = dequeue_pending_signal(ctx) {
            dequeued_signal
        } else {
            return;
        }
    };

    let sig_num = signal.num();
    match sig_action {
        SigAction::Ign => {
            debug!("Ignore signal {:?}", sig_num);
        }
        SigAction::User {
            handler_addr,
            flags,
            restorer_addr,
            mask,
        } => {
            if let Some(pre_syscall_ret) = syscall_restart
                && flags.contains(SigActionFlags::SA_RESTART)
            {
                #[cfg(target_arch = "x86_64")]
                const SYSCALL_INSTR_LEN: usize = 2; // syscall
                #[cfg(target_arch = "riscv64")]
                const SYSCALL_INSTR_LEN: usize = 4; // ecall
                #[cfg(target_arch = "loongarch64")]
                const SYSCALL_INSTR_LEN: usize = 4; // syscall

                user_ctx.set_syscall_ret(pre_syscall_ret);
                user_ctx
                    .set_instruction_pointer(user_ctx.instruction_pointer() - SYSCALL_INSTR_LEN);
            }

            if let Err(e) = handle_user_signal(
                ctx,
                sig_num,
                handler_addr,
                flags,
                restorer_addr,
                mask,
                restore_sig_mask.map(RestoreSigMaskGuard::into_mask),
                user_ctx,
                signal.to_info(),
            ) {
                debug!("Failed to handle user signal: {:?}", e);
                // If signal handling fails, the process should be terminated with SIGSEGV.
                // Reference: <https://elixir.bootlin.com/linux/v6.13/source/kernel/signal.c#L3082>
                do_exit_group(TermStatus::Killed(SIGSEGV));
            }
        }
        SigAction::Dfl if ctx.process.is_init_process() => {
            // From Linux man pages "kill(2)":
            // "The only signals that can be sent to process ID 1, the init process, are those for
            // which init has explicitly installed signal handlers."
        }
        SigAction::Dfl => {
            let sig_default_action = SigDefaultAction::from_signum(sig_num);
            debug!("sig_default_action = {:?}", sig_default_action);

            match sig_default_action {
                SigDefaultAction::Core | SigDefaultAction::Term => {
                    warn!(
                        "PID {}: terminating on signal {}",
                        ctx.process.pid(),
                        sig_num.sig_name()
                    );
                    // The signal terminates the current process. Therefore, we should exit here.
                    do_exit_group(TermStatus::Killed(sig_num));
                }
                SigDefaultAction::Ign => {}
                SigDefaultAction::Stop => ctx.process.stop(sig_num),
                SigDefaultAction::Cont => ctx.process.resume(),
            }
        }
    }
}

/// A guard that restores the signal mask on drop.
struct RestoreSigMaskGuard<'a> {
    ctx: &'a Context<'a>,
    mask: SigMask,
}

impl RestoreSigMaskGuard<'_> {
    /// Forgets the guard and returns the signal mask to restore.
    fn into_mask(self) -> SigMask {
        let mask = self.mask;
        // Assert that it's a `Copy` type. So we won't leak resources.
        let _ = self.mask;

        core::mem::forget(self);

        mask
    }
}

impl Drop for RestoreSigMaskGuard<'_> {
    fn drop(&mut self) {
        self.ctx.set_sig_mask(self.mask);
    }
}

fn dequeue_pending_signal(ctx: &Context) -> Option<(Box<dyn Signal>, SigAction)> {
    let posix_thread = ctx.posix_thread;

    let sig_dispositions = ctx.process.sig_dispositions().lock();
    let mut sig_dispositions = sig_dispositions.lock();

    let sig_mask = posix_thread.sig_mask();
    let (signal, sig_num, sig_action) = loop {
        let signal = ctx.dequeue_signal(&sig_mask)?;
        let sig_num = signal.num();
        let sig_action = sig_dispositions.get(sig_num);
        if sig_action.will_ignore(sig_num) {
            continue;
        }

        break (signal, sig_num, sig_action);
    };

    if let SigAction::User { flags, .. } = &sig_action
        && flags.contains(SigActionFlags::SA_RESETHAND)
    {
        // In Linux, SA_RESETHAND corresponds to SA_ONESHOT,
        // which means the user handler will be executed only once and then reset to the default.
        // Reference: <https://elixir.bootlin.com/linux/v6.0.9/source/kernel/signal.c#L2761>.
        sig_dispositions.set_default(sig_num);
    }

    debug!(
        "sig_num = {:?}, sig_name = {}, sig_action = {:#x?}",
        signal.num(),
        signal.num().sig_name(),
        sig_action
    );

    Some((signal, sig_action))
}

#[expect(clippy::too_many_arguments)]
pub fn handle_user_signal(
    ctx: &Context,
    sig_num: SigNum,
    handler_addr: Vaddr,
    flags: SigActionFlags,
    restorer_addr: Vaddr,
    mut mask: SigMask,
    mask_to_restore: Option<SigMask>,
    user_ctx: &mut UserContext,
    sig_info: siginfo_t,
) -> Result<()> {
    debug!("sig_num = {:?}, signame = {}", sig_num, sig_num.sig_name());
    debug!("handler_addr = 0x{:x}", handler_addr);
    debug!("flags = {:?}", flags);
    debug!("restorer_addr = 0x{:x}", restorer_addr);
    debug!("mask = {:?}, mask_to_restore = {:?}", mask, mask_to_restore);

    if flags.contains_unsupported_flag() {
        warn!("Unsupported signal flags: {:?}", flags);
    }

    if !flags.contains(SigActionFlags::SA_NODEFER) {
        // Add the current signal to `mask`.
        mask += sig_num;
    }

    // Block signals in `mask` while running the signal handler.
    let old_mask = ctx.posix_thread.sig_mask();
    let mask_to_restore = mask_to_restore.unwrap_or(old_mask);
    ctx.set_sig_mask(old_mask + mask);

    // Set up the signal stack.
    let mut stack_pointer = if let Some(sp) =
        use_alternate_signal_stack(flags, ctx.thread_local, user_ctx.stack_pointer())
    {
        sp as u64
    } else {
        // Just use the user stack.
        let sp = user_ctx.stack_pointer() as u64;

        // Prevent corruption of the current stack. Architectures like as x86-64 have red zones
        // that can be accessed below the SP. Signal handlers must not write to these zones.
        // FIXME: This may not be necessary for all architectures.
        sp.wrapping_sub(128)
    };

    let user_space = ctx.user_space();

    // 1. Write `siginfo_t`.
    stack_pointer = alloc_aligned_in_user_stack(
        stack_pointer,
        size_of::<siginfo_t>(),
        align_of::<siginfo_t>(),
    );
    user_space.write_val(stack_pointer as _, &sig_info)?;
    let siginfo_addr = stack_pointer;

    // 2. Write `ucontext_t`.

    // Save the current signal stack information.
    let uc_stack = {
        let mut sig_stack = ctx.thread_local.sig_stack().borrow_mut();
        let stack = stack_t::from(&*sig_stack);

        if sig_stack.flags().contains(SigStackFlags::SS_AUTODISARM) {
            sig_stack.reset();
        }

        stack
    };

    let mut ucontext = ucontext_t {
        uc_sigmask: mask_to_restore.into(),
        uc_stack,
        ..Default::default()
    };

    // Save general-purpose registers.
    ucontext.uc_mcontext.copy_user_regs_from(user_ctx);

    // Clone and reset the FPU context.
    let fpu_context = ctx.thread_local.fpu().clone_context();
    let fpu_context_bytes = fpu_context.as_bytes();
    ctx.thread_local.fpu().set_context(FpuContext::new());

    cfg_if::cfg_if! {
        if #[cfg(target_arch = "x86_64")] {
            // Align the FPU context address to the 64-byte boundary so that the
            // user program can use the XSAVE/XRSTOR instructions at that address,
            // if necessary.
            let fpu_context_addr =
                alloc_aligned_in_user_stack(stack_pointer, fpu_context_bytes.len(), 64);
            let ucontext_addr = alloc_aligned_in_user_stack(
                fpu_context_addr,
                size_of::<ucontext_t>(),
                align_of::<ucontext_t>(),
            );
            ucontext
                .uc_mcontext
                .set_fpu_context_addr(fpu_context_addr as _);

            const UC_FP_XSTATE: u64 = 1 << 0;
            ucontext.uc_flags = UC_FP_XSTATE;
        } else if #[cfg(target_arch = "riscv64")] {
            // Reference:
            // <https://elixir.bootlin.com/linux/v6.17.5/source/arch/riscv/include/uapi/asm/ptrace.h#L94-L98>,
            // <https://elixir.bootlin.com/linux/v6.17.5/source/arch/riscv/include/uapi/asm/ptrace.h#L69-L77>.
            const FP_STATE_SIZE: usize =
                size_of::<ostd::arch::cpu::context::QFpuContext>() + 3 * size_of::<u32>();

            let ucontext_addr = alloc_aligned_in_user_stack(
                stack_pointer,
                size_of::<ucontext_t>() + FP_STATE_SIZE,
                align_of::<ucontext_t>(),
            );
            let fpu_context_addr = (ucontext_addr as usize) + size_of::<ucontext_t>();
        } else if #[cfg(target_arch = "loongarch64")] {
            // FIXME: It seems that we need to allocate an `sctx_info` structure.
            // Reference: <https://elixir.bootlin.com/linux/v6.15.7/source/arch/loongarch/kernel/signal.c#L848>
            let ucontext_addr = alloc_aligned_in_user_stack(
                stack_pointer,
                size_of::<ucontext_t>() + fpu_context_bytes.len(),
                align_of::<ucontext_t>(),
            );
            // TODO: Set the flags in the context structure.
            // Reference: <https://elixir.bootlin.com/linux/v6.15.7/source/arch/loongarch/kernel/signal.c#L805>
            let fpu_context_addr = (ucontext_addr as usize) + size_of::<ucontext_t>();
        } else {
            compile_error!("unsupported target");
        }
    }

    user_space.write_bytes(fpu_context_addr as _, fpu_context_bytes)?;
    user_space.write_val(ucontext_addr as _, &ucontext)?;

    // Align the SP to the 16-byte boundary. This is required by the x86-64 ABI before calling any
    // function.
    stack_pointer = ucontext_addr.align_down(16);

    // 3. Write the address of the restorer code.
    let retaddr = if flags.contains(SigActionFlags::SA_RESTORER) {
        restorer_addr
    } else {
        cfg_if::cfg_if! {
            if #[cfg(target_arch = "riscv64")] {
                ctx.user_space().vmar().process_vm().vdso_base()
                    + crate::vdso::__VDSO_RT_SIGRETURN_OFFSET
            } else {
                // Note that this should already be rejected at the `rt_sigaction` system call.
                return_errno_with_message!(
                    Errno::EINVAL,
                    "this architecture currently requires SA_RESTORER"
                )
            }
        }
    };
    cfg_if::cfg_if! {
        if #[cfg(target_arch = "x86_64")] {
            stack_pointer = write_u64_to_user_stack(stack_pointer, retaddr as u64)?;
        } else if #[cfg(any(target_arch = "riscv64", target_arch = "loongarch64"))] {
            user_ctx.set_ra(retaddr);
        } else {
            compile_error!("unsupported target");
        }
    }

    debug!(
        "Before calling to signal handler: stack_pointer = 0x{:x}",
        stack_pointer
    );

    // 4. Set correct register values.
    user_ctx.set_instruction_pointer(handler_addr as _);
    user_ctx.set_stack_pointer(stack_pointer as usize);
    // Set parameters of the signal handler.
    if flags.contains(SigActionFlags::SA_SIGINFO) {
        user_ctx.set_arguments(sig_num, siginfo_addr as usize, ucontext_addr as usize);
    } else {
        user_ctx.set_arguments(sig_num, 0, 0);
    }
    // Perform CPU architecture-dependent logic.
    cfg_if::cfg_if! {
        if #[cfg(target_arch = "x86_64")] {
            // Clear the DF flag. This is to conform to x86-64 calling conventions.
            const X86_RFLAGS_DF: usize = 1 << 10; // Bit 10 is the DF flag.
            user_ctx.general_regs_mut().rflags &= !X86_RFLAGS_DF;
        }
    }

    Ok(())
}

/// Uses the alternate signal stack configured via `sigaltstack`.
///
/// If the current stack pointer `sp` is already within the alternate signal stack
/// or if the stack is disabled, this function returns `None`.
/// Otherwise, it returns the starting stack pointer of the alternate signal stack.
fn use_alternate_signal_stack(
    flags: SigActionFlags,
    thread_local: &ThreadLocal,
    sp: usize,
) -> Option<usize> {
    if !flags.contains(SigActionFlags::SA_ONSTACK) {
        return None;
    }

    let sig_stack = thread_local.sig_stack().borrow();

    if sig_stack.active_status(sp) != SigStackStatus::Inactive {
        return None;
    }

    // Overflow checks are done in the `sigaltstack` system call.
    Some(sig_stack.base() + sig_stack.size())
}

/// Writes a `u64` integer to the user's stack.
#[cfg(target_arch = "x86_64")]
fn write_u64_to_user_stack(sp: u64, value: u64) -> Result<u64> {
    use crate::current_userspace;

    let sp = sp.wrapping_sub(size_of::<u64>() as u64);
    current_userspace!().write_val(sp as _, &value)?;
    Ok(sp)
}

/// Allocates `size` bytes on the user's stack.
///
/// The allocation will be aligned to `align`, which must be a power of two.
fn alloc_aligned_in_user_stack(sp: u64, size: usize, align: usize) -> u64 {
    sp.wrapping_sub(size as u64).align_down(align as u64)
}
