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
pub use events::{SigEvents, SigEventsFilter};
use ostd::{cpu::UserContext, user::UserContextApi};
pub use pause::{with_signal_blocked, Pause};
pub use poll::{PollAdaptor, PollHandle, Pollable, Pollee, Poller};
use sig_action::{SigAction, SigActionFlags, SigDefaultAction};
use sig_mask::SigMask;
use sig_num::SigNum;
pub use sig_stack::{SigStack, SigStackFlags};

use super::posix_thread::PosixThread;
use crate::{
    current_userspace,
    prelude::*,
    process::{do_exit_group, TermStatus},
};

pub trait SignalContext {
    /// Set signal handler arguments
    fn set_arguments(&mut self, sig_num: SigNum, siginfo_addr: usize, ucontext_addr: usize);
}

// TODO: This interface of this method is error prone.
// The method takes an argument for the current thread to optimize its efficiency.
/// Handle pending signal for current process.
pub fn handle_pending_signal(user_ctx: &mut UserContext, ctx: &Context) -> Result<()> {
    // We first deal with signal in current thread, then signal in current process.
    let posix_thread = ctx.posix_thread;
    let signal = {
        let sig_mask = posix_thread.sig_mask().load(Ordering::Relaxed);
        if let Some(signal) = posix_thread.dequeue_signal(&sig_mask) {
            signal
        } else {
            return Ok(());
        }
    };

    let sig_num = signal.num();
    trace!("sig_num = {:?}, sig_name = {}", sig_num, sig_num.sig_name());
    let current = posix_thread.process();
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
            if flags.contains(SigActionFlags::SA_RESETHAND) {
                // In Linux, SA_RESETHAND corresponds to SA_ONESHOT,
                // which means the user handler will be executed only once and then reset to the default.
                // Refer to https://elixir.bootlin.com/linux/v6.0.9/source/kernel/signal.c#L2761.
                sig_dispositions.set_default(sig_num);
            }

            drop(sig_dispositions);

            handle_user_signal(
                ctx,
                sig_num,
                handler_addr,
                flags,
                restorer_addr,
                mask,
                user_ctx,
                signal.to_info(),
            )?
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
                SigDefaultAction::Stop => {
                    let _ = ctx.thread.stop();
                }
                SigDefaultAction::Cont => {
                    let _ = ctx.thread.resume();
                }
            }
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
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
    // FIXME: How to respect flags?
    if flags.contains_unsupported_flag() {
        warn!("Unsupported Signal flags: {:?}", flags);
    }

    if !flags.contains(SigActionFlags::SA_NODEFER) {
        // add current signal to mask
        mask += sig_num;
    }

    // block signals in sigmask when running signal handler
    let old_mask = ctx.posix_thread.sig_mask().load(Ordering::Relaxed);
    ctx.posix_thread
        .sig_mask()
        .store(old_mask + mask, Ordering::Relaxed);

    // Set up signal stack.
    let mut stack_pointer = if let Some(sp) = use_alternate_signal_stack(ctx.posix_thread) {
        sp as u64
    } else {
        // just use user stack
        user_ctx.stack_pointer() as u64
    };

    // To avoid corrupting signal stack, we minus 128 first.
    stack_pointer -= 128;

    let user_space = ctx.user_space();

    // 1. write siginfo_t
    stack_pointer -= mem::size_of::<siginfo_t>() as u64;
    user_space.write_val(stack_pointer as _, &sig_info)?;
    let siginfo_addr = stack_pointer;

    // 2. write ucontext_t.
    stack_pointer = alloc_aligned_in_user_stack(stack_pointer, mem::size_of::<ucontext_t>(), 16)?;
    let mut ucontext = ucontext_t {
        uc_sigmask: mask.into(),
        ..Default::default()
    };
    ucontext
        .uc_mcontext
        .inner
        .gp_regs
        .copy_from_raw(user_ctx.general_regs());
    let mut sig_context = ctx.posix_thread.sig_context().lock();
    if let Some(sig_context_addr) = *sig_context {
        ucontext.uc_link = sig_context_addr;
    } else {
        ucontext.uc_link = 0;
    }
    // TODO: store fp regs in ucontext
    user_space.write_val(stack_pointer as _, &ucontext)?;
    let ucontext_addr = stack_pointer;
    // Store the ucontext addr in sig context of current thread.
    *sig_context = Some(ucontext_addr as Vaddr);

    // 3. Set the address of the trampoline code.
    if flags.contains(SigActionFlags::SA_RESTORER) {
        // If contains SA_RESTORER flag, trampoline code is provided by libc in restorer_addr.
        // We just store restorer_addr on user stack to allow user code just to trampoline code.
        stack_pointer = write_u64_to_user_stack(stack_pointer, restorer_addr as u64)?;
        trace!("After set restorer addr: user_rsp = 0x{:x}", stack_pointer);
    } else {
        // Otherwise we create a trampoline.
        // FIXME: This may cause problems if we read old_context from rsp.
        const TRAMPOLINE: &[u8] = &[
            0xb8, 0x0f, 0x00, 0x00, 0x00, // mov eax, 15(syscall number of rt_sigreturn)
            0x0f, 0x05, // syscall (call rt_sigreturn)
            0x90, // nop (for alignment)
        ];
        stack_pointer -= TRAMPOLINE.len() as u64;
        let trampoline_rip = stack_pointer;
        user_space.write_bytes(stack_pointer as Vaddr, &mut VmReader::from(TRAMPOLINE))?;
        stack_pointer = write_u64_to_user_stack(stack_pointer, trampoline_rip)?;
    }

    // 4. Set correct register values
    user_ctx.set_instruction_pointer(handler_addr as _);
    user_ctx.set_stack_pointer(stack_pointer as usize);
    // parameters of signal handler
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
fn use_alternate_signal_stack(posix_thread: &PosixThread) -> Option<usize> {
    let mut sig_stack = posix_thread.sig_stack().lock();
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
