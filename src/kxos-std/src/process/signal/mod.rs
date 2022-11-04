pub mod c_types;
pub mod constants;
pub mod sig_action;
pub mod sig_disposition;
pub mod sig_mask;
pub mod sig_num;
pub mod sig_queues;
pub mod signals;

use kxos_frame::{cpu::CpuContext, task::Task};

use self::sig_mask::SigMask;
use self::sig_num::SigNum;
use crate::memory::{write_bytes_to_user, write_val_to_user};
use crate::process::signal::sig_action::SigActionFlags;
use crate::{
    prelude::*,
    process::signal::sig_action::{SigAction, SigDefaultAction},
};

/// Handle pending signal for current process
pub fn handle_pending_signal(context: &mut CpuContext) {
    let current = current!();
    let pid = current.pid();
    let process_name = current.filename().unwrap();
    let sig_queues = current.sig_queues();
    let mut sig_queues_guard = sig_queues.lock();
    let sig_mask = current.sig_mask().lock().clone();
    if let Some(signal) = sig_queues_guard.dequeue(&sig_mask) {
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
            } => handle_user_signal_handler(
                sig_num,
                handler_addr,
                flags,
                restorer_addr,
                mask,
                context,
            ),
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
                        current.exit(1);
                        // We should exit current here, since we cannot restore a valid status from trap now.
                        Task::current().exit();
                    }
                    SigDefaultAction::Ign => {}
                    SigDefaultAction::Stop => {
                        let mut status_guard = current.status().lock();
                        if status_guard.is_runnable() {
                            status_guard.set_suspend();
                        } else {
                            panic!("Try to suspend a not running process.")
                        }
                        drop(status_guard);
                    }
                    SigDefaultAction::Cont => {
                        let mut status_guard = current.status().lock();
                        if status_guard.is_suspend() {
                            status_guard.set_runnable();
                        } else {
                            panic!("Try to continue a not suspended process.")
                        }
                        drop(status_guard);
                    }
                }
            }
        }
    }
}

pub fn handle_user_signal_handler(
    sig_num: SigNum,
    handler_addr: Vaddr,
    flags: SigActionFlags,
    restorer_addr: Vaddr,
    mask: SigMask,
    context: &mut CpuContext,
) {
    debug!("sig_num = {:?}", sig_num);
    debug!("handler_addr = 0x{:x}", handler_addr);
    debug!("flags = {:?}", flags);
    debug!("restorer_addr = 0x{:x}", restorer_addr);
    // FIXME: How to respect flags
    if flags.intersects(!(SigActionFlags::SA_RESTART | SigActionFlags::SA_RESTORER)) {
        panic!("Unsupported Signal flags");
    }
    let current = current!();
    // block signals in sigmask when running signal handler
    current.sig_mask().lock().block(mask.as_u64());
    // store context in current process
    let sig_context = SigContext::new(context.clone(), mask);
    *(current.sig_context().lock()) = Some(sig_context);
    // set up signal stack in user stack
    let mut user_rsp = context.gp_regs.rsp;
    // avoid corrupt user stack, we minus 128 first.
    user_rsp = user_rsp - 128;
    // Copy the trampoline code.
    if flags.contains(SigActionFlags::SA_RESTORER) {
        // If contains SA_RESTORER flag, trampoline code is provided by libc in restorer_addr.
        // We just store restorer_addr on user stack to allow user code just to trampoline code.
        user_rsp = write_u64_to_user_stack(user_rsp, restorer_addr as u64);
    } else {
        // Otherwise we create
        const TRAMPOLINE: &[u8] = &[
            0xb8, 0x0f, 0x00, 0x00, 0x00, // mov eax, 15(syscall number of rt_sigreturn)
            0x0f, 0x05, // syscall (call rt_sigreturn)
            0x90, // nop (for alignment)
        ];
        user_rsp = user_rsp - TRAMPOLINE.len() as u64;
        let trampoline_rip = user_rsp;
        write_bytes_to_user(user_rsp as Vaddr, TRAMPOLINE);
        user_rsp = write_u64_to_user_stack(user_rsp, trampoline_rip);
    }
    context.gp_regs.rip = handler_addr as _;
    context.gp_regs.rsp = user_rsp;
    // parameters of signal handler
    context.gp_regs.rdi = sig_num.as_u8() as u64; // signal number
    context.gp_regs.rsi = 0; // siginfo_t* siginfo
    context.gp_regs.rdx = 0; // void* ctx
}

fn write_u64_to_user_stack(rsp: u64, value: u64) -> u64 {
    let rsp = rsp - 8;
    write_val_to_user(rsp as Vaddr, &value);
    rsp
}

/// Used to store process context before running signal handler.
/// In rt_sigreturn, this context is used to restore process context.
#[derive(Debug, Clone, Copy)]
pub struct SigContext {
    cpu_context: CpuContext,
    sig_mask: SigMask,
}

impl SigContext {
    pub const fn new(cpu_context: CpuContext, sig_mask: SigMask) -> SigContext {
        Self {
            cpu_context,
            sig_mask,
        }
    }

    pub fn cpu_context(&self) -> &CpuContext {
        &self.cpu_context
    }

    pub fn sig_mask(&self) -> &SigMask {
        &self.sig_mask
    }
}
