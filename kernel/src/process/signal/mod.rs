// SPDX-License-Identifier: MPL-2.0

pub mod c_types;
pub mod constants;
mod events;
mod pause;
mod poll;
pub mod sig_action;
pub mod sig_disposition;
pub mod sig_mask;
pub mod sig_num;
pub mod sig_queues;
mod sig_stack;
pub mod signals;

use core::{mem, sync::atomic::Ordering};

use align_ext::AlignExt;
use c_types::{siginfo_t, ucontext_t};
use constants::SIGSEGV;
pub use events::{SigEvents, SigEventsFilter};
use ostd::{
    cpu::context::{FpuContext, UserContext},
    user::UserContextApi,
};
pub use pause::{with_sigmask_changed, Pause};
pub use poll::{PollAdaptor, PollHandle, Pollable, Pollee, Poller};
use sig_action::{SigAction, SigActionFlags, SigDefaultAction};
use sig_mask::SigMask;
use sig_num::SigNum;
pub use sig_stack::{SigStack, SigStackFlags};

use super::posix_thread::ThreadLocal;
use crate::{
    cpu::LinuxAbi,
    current_userspace,
    prelude::*,
    process::{posix_thread::do_exit_group, TermStatus},
};

pub trait SignalContext {
    /// Set signal handler arguments
    fn set_arguments(&mut self, sig_num: SigNum, siginfo_addr: usize, ucontext_addr: usize);
}

// TODO: This interface of this method is error prone.
// The method takes an argument for the current thread to optimize its efficiency.
/// Handle pending signal for current process.
pub fn handle_pending_signal(
    user_ctx: &mut UserContext,
    ctx: &Context,
    syscall_number: Option<usize>,
) {
    let syscall_restart = if let Some(syscall_number) = syscall_number
        && user_ctx.syscall_ret() == -(Errno::ERESTARTSYS as i32) as usize
    {
        // We should never return `ERESTARTSYS` to the userspace.
        user_ctx.set_syscall_ret(-(Errno::EINTR as i32) as usize);
        Some(syscall_number)
    } else {
        None
    };

    let posix_thread = ctx.posix_thread;
    let current = ctx.process;

    let signal = {
        let sig_mask = posix_thread.sig_mask().load(Ordering::Relaxed);
        if let Some(signal) = posix_thread.dequeue_signal(&sig_mask) {
            signal
        } else {
            return;
        }
    };
    let sig_num = signal.num();
    trace!("sig_num = {:?}, sig_name = {}", sig_num, sig_num.sig_name());

    let mut sig_dispositions = current.sig_dispositions().lock();

    let sig_action = sig_dispositions.get(sig_num);
    trace!("sig action: {:x?}", sig_action);

    match sig_action {
        SigAction::Ign => {
            trace!("Ignore signal {:?}", sig_num);
        }
        SigAction::User {
            handler_addr,
            flags,
            restorer_addr,
            mask,
        } => {
            if let Some(syscall_number) = syscall_restart
                && flags.contains(SigActionFlags::SA_RESTART)
            {
                #[cfg(target_arch = "x86_64")]
                const SYSCALL_INSTR_LEN: usize = 2; // syscall
                #[cfg(target_arch = "riscv64")]
                const SYSCALL_INSTR_LEN: usize = 4; // ecall
                #[cfg(target_arch = "loongarch64")]
                const SYSCALL_INSTR_LEN: usize = 4; // syscall

                user_ctx.set_syscall_num(syscall_number);
                user_ctx
                    .set_instruction_pointer(user_ctx.instruction_pointer() - SYSCALL_INSTR_LEN);
            }

            if flags.contains(SigActionFlags::SA_RESETHAND) {
                // In Linux, SA_RESETHAND corresponds to SA_ONESHOT,
                // which means the user handler will be executed only once and then reset to the default.
                // Refer to https://elixir.bootlin.com/linux/v6.0.9/source/kernel/signal.c#L2761.
                sig_dispositions.set_default(sig_num);
            }

            drop(sig_dispositions);
            if let Err(e) = handle_user_signal(
                ctx,
                sig_num,
                handler_addr,
                flags,
                restorer_addr,
                mask,
                user_ctx,
                signal.to_info(),
            ) {
                debug!("Failed to handle user signal: {:?}", e);
                // If signal handling fails, the process should be terminated with SIGSEGV.
                // Ref: <https://elixir.bootlin.com/linux/v6.13/source/kernel/signal.c#L3082>
                do_exit_group(TermStatus::Killed(SIGSEGV));
            }
        }
        SigAction::Dfl => {
            drop(sig_dispositions);

            let sig_default_action = SigDefaultAction::from_signum(sig_num);
            trace!("sig_default_action: {:?}", sig_default_action);
            match sig_default_action {
                SigDefaultAction::Core | SigDefaultAction::Term => {
                    warn!(
                        "{:?}: terminating on signal {}",
                        current.executable_path(),
                        sig_num.sig_name()
                    );
                    // We should exit current here, since we cannot restore a valid status from trap now.
                    do_exit_group(TermStatus::Killed(sig_num));
                }
                SigDefaultAction::Ign => {}
                SigDefaultAction::Stop => ctx.process.stop(sig_num),
                SigDefaultAction::Cont => ctx.process.resume(),
            }
        }
    }
}

#[expect(clippy::too_many_arguments)]
pub fn handle_user_signal(
    ctx: &Context,
    sig_num: SigNum,
    handler_addr: Vaddr,
    flags: SigActionFlags,
    restorer_addr: Vaddr,
    mut mask: SigMask,
    user_ctx: &mut UserContext,
    sig_info: siginfo_t,
) -> Result<()> {
    debug!("sig_num = {:?}, signame = {}", sig_num, sig_num.sig_name());
    debug!("handler_addr = 0x{:x}", handler_addr);
    debug!("flags = {:?}", flags);
    debug!("restorer_addr = 0x{:x}", restorer_addr);

    if flags.contains_unsupported_flag() {
        warn!("Unsupported Signal flags: {:?}", flags);
    }

    if !flags.contains(SigActionFlags::SA_NODEFER) {
        // Add current signal to mask
        mask += sig_num;
    }

    // Block signals in sigmask when running signal handler
    let old_mask = ctx.posix_thread.sig_mask().load(Ordering::Relaxed);
    ctx.posix_thread
        .sig_mask()
        .store(old_mask + mask, Ordering::Relaxed);

    // Set up signal stack.
    let mut stack_pointer = if let Some(sp) = use_alternate_signal_stack(ctx.thread_local) {
        sp as u64
    } else {
        // Just use user stack
        user_ctx.stack_pointer() as u64
    };

    // To avoid corrupting signal stack, we minus 128 first.
    stack_pointer -= 128;

    let user_space = ctx.user_space();

    // 1. Write siginfo_t
    stack_pointer -= mem::size_of::<siginfo_t>() as u64;
    user_space.write_val(stack_pointer as _, &sig_info)?;
    let siginfo_addr = stack_pointer;

    // 2. Write ucontext_t.

    let mut ucontext = ucontext_t {
        uc_sigmask: mask.into(),
        ..Default::default()
    };
    ucontext.uc_mcontext.copy_user_regs_from(user_ctx);
    let sig_context = ctx.thread_local.sig_context().get();
    if let Some(sig_context_addr) = sig_context {
        ucontext.uc_link = sig_context_addr;
    } else {
        ucontext.uc_link = 0;
    }

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
                alloc_aligned_in_user_stack(stack_pointer, fpu_context_bytes.len(), 64)?;
            let ucontext_addr = alloc_aligned_in_user_stack(
                fpu_context_addr,
                size_of::<ucontext_t>(),
                align_of::<ucontext_t>(),
            )?;
            ucontext
                .uc_mcontext
                .set_fpu_context_addr(fpu_context_addr as _);

            const UC_FP_XSTATE: u64 = 1 << 0;
            ucontext.uc_flags = UC_FP_XSTATE;
        } else if #[cfg(target_arch = "riscv64")] {
            let ucontext_addr = alloc_aligned_in_user_stack(
                stack_pointer,
                size_of::<ucontext_t>() + fpu_context_bytes.len(),
                align_of::<ucontext_t>(),
            )?;
            let fpu_context_addr = (ucontext_addr as usize) + size_of::<ucontext_t>();
        } else if #[cfg(target_arch = "loongarch64")] {
            // FIXME: It seems that we still need to allocate an sctx_info struct
            // Reference: <https://elixir.bootlin.com/linux/v6.15.7/source/arch/loongarch/kernel/signal.c#L848>
            let ucontext_addr = alloc_aligned_in_user_stack(
                stack_pointer,
                size_of::<ucontext_t>() + fpu_context_bytes.len(),
                align_of::<ucontext_t>(),
            )?;
            // TODO: Set the `SigContext`.flags
            // Reference: <https://elixir.bootlin.com/linux/v6.15.7/source/arch/loongarch/kernel/signal.c#L805>
            let fpu_context_addr = (ucontext_addr as usize) + size_of::<ucontext_t>();
        } else {
            compile_error!("unsupported target");
        }
    }

    let mut fpu_context_reader = VmReader::from(fpu_context.as_bytes());
    user_space.write_bytes(fpu_context_addr as _, &mut fpu_context_reader)?;

    user_space.write_val(ucontext_addr as _, &ucontext)?;
    // Store the ucontext addr in sig context of current thread.
    ctx.thread_local
        .sig_context()
        .set(Some(ucontext_addr as Vaddr));

    // 3. Write the address of the restorer code.
    stack_pointer = ucontext_addr;
    if flags.contains(SigActionFlags::SA_RESTORER) {
        // If the SA_RESTORER flag is present, the restorer code address is provided by the user.
        stack_pointer = write_u64_to_user_stack(stack_pointer, restorer_addr as u64)?;
        trace!(
            "After writing restorer addr: user_rsp = 0x{:x}",
            stack_pointer
        );
    }

    // 4. Set correct register values
    user_ctx.set_instruction_pointer(handler_addr as _);
    user_ctx.set_stack_pointer(stack_pointer as usize);
    // Parameters of signal handler
    if flags.contains(SigActionFlags::SA_SIGINFO) {
        user_ctx.set_arguments(sig_num, siginfo_addr as usize, ucontext_addr as usize);
    } else {
        user_ctx.set_arguments(sig_num, 0, 0);
    }
    // CPU architecture-dependent logic
    cfg_if::cfg_if! {
        if #[cfg(target_arch = "x86_64")] {
            // Clear `DF` flag for C function entry to conform to x86-64 calling convention.
            // Bit 10 is the DF flag.
            const X86_RFLAGS_DF: usize = 1 << 10;
            user_ctx.general_regs_mut().rflags &= !X86_RFLAGS_DF;
        }
    }

    Ok(())
}

/// Use an alternate signal stack, which was installed by sigaltstack.
/// It the stack is already active, we just increase the handler counter and return None, since
/// the stack pointer can be read from context.
/// It the stack is not used by any handler, we will return the new sp in alternate signal stack.
fn use_alternate_signal_stack(thread_local: &ThreadLocal) -> Option<usize> {
    let mut sig_stack = thread_local.sig_stack().borrow_mut();
    let sig_stack = (*sig_stack).as_mut()?;

    if sig_stack.is_disabled() {
        return None;
    }

    if sig_stack.is_active() {
        // The stack is already active, so we just use sp in context.
        sig_stack.increase_handler_counter();
        return None;
    }

    sig_stack.increase_handler_counter();

    // Make sp align at 16. FIXME: is this required?
    let stack_pointer = (sig_stack.base() + sig_stack.size()).align_down(16);
    Some(stack_pointer)
}

fn write_u64_to_user_stack(rsp: u64, value: u64) -> Result<u64> {
    let rsp = rsp - 8;
    current_userspace!().write_val(rsp as Vaddr, &value)?;
    Ok(rsp)
}

/// alloc memory of size on user stack, the return address should respect the align argument.
fn alloc_aligned_in_user_stack(rsp: u64, size: usize, align: usize) -> Result<u64> {
    if !align.is_power_of_two() {
        return_errno_with_message!(Errno::EINVAL, "align must be power of two");
    }
    let start = (rsp - size as u64).align_down(align as u64);
    Ok(start)
}
