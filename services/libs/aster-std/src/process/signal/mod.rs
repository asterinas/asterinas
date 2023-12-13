pub mod c_types;
pub mod constants;
mod events;
mod pauser;
mod poll;
pub mod sig_action;
pub mod sig_disposition;
pub mod sig_mask;
pub mod sig_num;
pub mod sig_queues;
mod sig_stack;
pub mod signals;

pub use events::{SigEvents, SigEventsFilter};
pub use pauser::Pauser;
pub use poll::{Pollee, Poller};
pub use sig_stack::{SigStack, SigStackFlags};

use align_ext::AlignExt;
use aster_frame::cpu::UserContext;
use aster_frame::task::Task;
use core::mem;

use super::posix_thread::{PosixThread, PosixThreadExt};
use crate::prelude::*;
use crate::process::{do_exit_group, TermStatus};
use crate::util::{write_bytes_to_user, write_val_to_user};
use c_types::{siginfo_t, ucontext_t};
use sig_action::{SigAction, SigActionFlags, SigDefaultAction};
use sig_mask::SigMask;
use sig_num::SigNum;

/// Handle pending signal for current process
pub fn handle_pending_signal(context: &mut UserContext) -> Result<()> {
    let current = current!();
    let current_thread = current_thread!();

    // We first deal with signal in current thread, then signal in current process.
    let signal = {
        let posix_thread = current_thread.as_posix_thread().unwrap();
        let sig_mask = *posix_thread.sig_mask().lock();
        if let Some(signal) = posix_thread.dequeue_signal(&sig_mask) {
            signal
        } else {
            return Ok(());
        }
    };

    let sig_num = signal.num();
    trace!("sig_num = {:?}, sig_name = {}", sig_num, sig_num.sig_name());
    let sig_action = current.sig_dispositions().lock().get(sig_num);
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
        } => handle_user_signal(
            sig_num,
            handler_addr,
            flags,
            restorer_addr,
            mask,
            context,
            signal.to_info(),
        )?,
        SigAction::Dfl => {
            let sig_default_action = SigDefaultAction::from_signum(sig_num);
            trace!("sig_default_action: {:?}", sig_default_action);
            match sig_default_action {
                SigDefaultAction::Core | SigDefaultAction::Term => {
                    warn!(
                        "{:?}: terminating on signal {}",
                        current.executable_path(),
                        sig_num.sig_name()
                    );
                    do_exit_group(TermStatus::Killed(sig_num));
                    // We should exit current here, since we cannot restore a valid status from trap now.
                    Task::current().exit();
                }
                SigDefaultAction::Ign => {}
                SigDefaultAction::Stop => {
                    let mut status = current_thread.status().lock();
                    if status.is_running() {
                        status.set_stopped();
                    }
                    drop(status);
                }
                SigDefaultAction::Cont => {
                    let mut status = current_thread.status().lock();
                    if status.is_stopped() {
                        status.set_running();
                    }
                    drop(status);
                }
            }
        }
    }
    Ok(())
}

pub fn handle_user_signal(
    sig_num: SigNum,
    handler_addr: Vaddr,
    flags: SigActionFlags,
    restorer_addr: Vaddr,
    mut mask: SigMask,
    context: &mut UserContext,
    sig_info: siginfo_t,
) -> Result<()> {
    debug!("sig_num = {:?}, signame = {}", sig_num, sig_num.sig_name());
    debug!("handler_addr = 0x{:x}", handler_addr);
    debug!("flags = {:?}", flags);
    debug!("restorer_addr = 0x{:x}", restorer_addr);
    // FIXME: How to respect flags?
    if flags.contains_unsupported_flag() {
        println!("flags = {:?}", flags);
        panic!("Unsupported Signal flags");
    }
    if !flags.contains(SigActionFlags::SA_NODEFER) {
        // add current signal to mask
        let current_mask = SigMask::from(sig_num);
        mask.block(current_mask.as_u64());
    }
    let current_thread = current_thread!();
    let posix_thread = current_thread.as_posix_thread().unwrap();
    // block signals in sigmask when running signal handler
    posix_thread.sig_mask().lock().block(mask.as_u64());

    // Set up signal stack.
    let mut stack_pointer = if let Some(sp) = use_alternate_signal_stack(posix_thread) {
        sp as u64
    } else {
        // just use user stack
        context.rsp() as u64
    };

    // To avoid corrupting signal stack, we minus 128 first.
    stack_pointer -= 128;

    // 1. write siginfo_t
    stack_pointer -= mem::size_of::<siginfo_t>() as u64;
    write_val_to_user(stack_pointer as _, &sig_info)?;
    let siginfo_addr = stack_pointer;

    // 2. write ucontext_t.
    stack_pointer = alloc_aligned_in_user_stack(stack_pointer, mem::size_of::<ucontext_t>(), 16)?;
    let mut ucontext = ucontext_t {
        uc_sigmask: mask.as_u64(),
        ..Default::default()
    };
    ucontext.uc_mcontext.inner.gp_regs = *context.general_regs();
    let mut sig_context = posix_thread.sig_context().lock();
    if let Some(sig_context_addr) = *sig_context {
        ucontext.uc_link = sig_context_addr;
    } else {
        ucontext.uc_link = 0;
    }
    // TODO: store fp regs in ucontext
    write_val_to_user(stack_pointer as _, &ucontext)?;
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
        write_bytes_to_user(stack_pointer as Vaddr, TRAMPOLINE)?;
        stack_pointer = write_u64_to_user_stack(stack_pointer, trampoline_rip)?;
    }
    // 4. Set correct register values
    context.set_rip(handler_addr as _);
    context.set_rsp(stack_pointer as usize);
    // parameters of signal handler
    context.set_rdi(sig_num.as_u8() as usize); // signal number
    if flags.contains(SigActionFlags::SA_SIGINFO) {
        context.set_rsi(siginfo_addr as usize); // siginfo_t* siginfo
        context.set_rdx(ucontext_addr as usize); // void* ctx
    } else {
        context.set_rsi(0);
        context.set_rdx(0);
    }

    Ok(())
}

/// Use an alternate signal stack, which was installed by sigaltstack.
/// It the stack is already active, we just increase the handler counter and return None, since
/// the stack pointer can be read from context.
/// It the stack is not used by any handler, we will return the new sp in alternate signal stack.
fn use_alternate_signal_stack(posix_thread: &PosixThread) -> Option<usize> {
    let mut sig_stack = posix_thread.sig_stack().lock();

    let Some(sig_stack) = &mut *sig_stack else {
        return None;
    };

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
    write_val_to_user(rsp as Vaddr, &value)?;
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
