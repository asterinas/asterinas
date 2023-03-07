pub mod c_types;
pub mod constants;
pub mod sig_action;
pub mod sig_disposition;
pub mod sig_mask;
pub mod sig_num;
pub mod sig_queues;
pub mod signals;

use core::mem;

use jinux_frame::AlignExt;
use jinux_frame::{cpu::CpuContext, task::Task};

use self::c_types::siginfo_t;
use self::sig_mask::SigMask;
use self::sig_num::SigNum;
use crate::current_thread;
use crate::process::posix_thread::posix_thread_ext::PosixThreadExt;
use crate::process::signal::c_types::ucontext_t;
use crate::process::signal::sig_action::SigActionFlags;
use crate::util::{write_bytes_to_user, write_val_to_user};
use crate::{
    prelude::*,
    process::signal::sig_action::{SigAction, SigDefaultAction},
};

/// Handle pending signal for current process
pub fn handle_pending_signal(context: &mut CpuContext) -> Result<()> {
    let current = current!();
    let current_thread = current_thread!();
    let posix_thread = current_thread.as_posix_thread().unwrap();
    let pid = current.pid();
    let process_name = current.filename().unwrap();
    let sig_mask = posix_thread.sig_mask().lock().clone();
    let mut thread_sig_queues = posix_thread.sig_queues().lock();
    let mut proc_sig_queues = current.sig_queues().lock();
    // We first deal with signal in current thread, then signal in current process.
    let signal = if let Some(signal) = thread_sig_queues.dequeue(&sig_mask) {
        Some(signal)
    } else if let Some(signal) = proc_sig_queues.dequeue(&sig_mask) {
        Some(signal)
    } else {
        None
    };
    if let Some(signal) = signal {
        let sig_num = signal.num();
        debug!("sig_num = {:?}, sig_name = {}", sig_num, sig_num.sig_name());
        let sig_action = current.sig_dispositions().lock().get(sig_num);
        debug!("sig action: {:x?}", sig_action);
        match sig_action {
            SigAction::Ign => {
                debug!("Ignore signal {:?}", sig_num);
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
                debug!("sig_default_action: {:?}", sig_default_action);
                match sig_default_action {
                    SigDefaultAction::Core | SigDefaultAction::Term => {
                        error!(
                            "{:?}: terminating on signal {}",
                            process_name,
                            sig_num.sig_name()
                        );
                        // FIXME: How to set correct status if process is terminated
                        current.exit_group(1);
                        // We should exit current here, since we cannot restore a valid status from trap now.
                        Task::current().exit();
                    }
                    SigDefaultAction::Ign => {}
                    SigDefaultAction::Stop => {
                        let mut status = current_thread.status().lock();
                        if status.is_running() {
                            status.set_stopped();
                        } else {
                            panic!("Try to suspend a not running process.")
                        }
                        drop(status);
                    }
                    SigDefaultAction::Cont => {
                        let mut status = current_thread.status().lock();
                        if status.is_stopped() {
                            status.set_running();
                        } else {
                            panic!("Try to continue a not suspended process.")
                        }
                        drop(status);
                    }
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
    context: &mut CpuContext,
    sig_info: siginfo_t,
) -> Result<()> {
    debug!("sig_num = {:?}", sig_num);
    debug!("handler_addr = 0x{:x}", handler_addr);
    debug!("flags = {:?}", flags);
    debug!("restorer_addr = 0x{:x}", restorer_addr);
    // FIXME: How to respect flags?
    if flags.contains_unsupported_flag() {
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

    // set up signal stack in user stack. // avoid corrupt user stack, we minus 128 first.
    let mut user_rsp = context.gp_regs.rsp;
    user_rsp = user_rsp - 128;

    // 1. write siginfo_t
    user_rsp = user_rsp - mem::size_of::<siginfo_t>() as u64;
    write_val_to_user(user_rsp as _, &sig_info)?;
    let siginfo_addr = user_rsp;
    // debug!("siginfo_addr = 0x{:x}", siginfo_addr);

    // 2. write ucontext_t.
    user_rsp = alloc_aligned_in_user_stack(user_rsp, mem::size_of::<ucontext_t>(), 16)?;
    let mut ucontext = ucontext_t::default();
    ucontext.uc_sigmask = mask.as_u64();
    ucontext.uc_mcontext.inner.gp_regs = context.gp_regs;
    let mut sig_context = posix_thread.sig_context().lock();
    if let Some(sig_context_addr) = *sig_context {
        ucontext.uc_link = sig_context_addr;
    } else {
        ucontext.uc_link = 0;
    }
    // TODO: store fp regs in ucontext
    write_val_to_user(user_rsp as _, &ucontext)?;
    let ucontext_addr = user_rsp;
    // Store the ucontext addr in sig context of current process.
    *sig_context = Some(ucontext_addr as Vaddr);
    // current.sig_context().lock().push_back(ucontext_addr as _);

    // 3. Set the address of the trampoline code.
    if flags.contains(SigActionFlags::SA_RESTORER) {
        // If contains SA_RESTORER flag, trampoline code is provided by libc in restorer_addr.
        // We just store restorer_addr on user stack to allow user code just to trampoline code.
        user_rsp = write_u64_to_user_stack(user_rsp, restorer_addr as u64)?;
        debug!("After set restorer addr: user_rsp = 0x{:x}", user_rsp);
    } else {
        // Otherwise we create a trampoline.
        // FIXME: This may cause problems if we read old_context from rsp.
        const TRAMPOLINE: &[u8] = &[
            0xb8, 0x0f, 0x00, 0x00, 0x00, // mov eax, 15(syscall number of rt_sigreturn)
            0x0f, 0x05, // syscall (call rt_sigreturn)
            0x90, // nop (for alignment)
        ];
        user_rsp = user_rsp - TRAMPOLINE.len() as u64;
        let trampoline_rip = user_rsp;
        write_bytes_to_user(user_rsp as Vaddr, TRAMPOLINE)?;
        user_rsp = write_u64_to_user_stack(user_rsp, trampoline_rip)?;
    }
    // 4. Set correct register values
    context.gp_regs.rip = handler_addr as _;
    context.gp_regs.rsp = user_rsp;
    // parameters of signal handler
    context.gp_regs.rdi = sig_num.as_u8() as u64; // signal number
    if flags.contains(SigActionFlags::SA_SIGINFO) {
        context.gp_regs.rsi = siginfo_addr; // siginfo_t* siginfo
        context.gp_regs.rdx = ucontext_addr; // void* ctx
    } else {
        context.gp_regs.rsi = 0;
        context.gp_regs.rdx = 0;
    }

    Ok(())
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
