// SPDX-License-Identifier: MPL-2.0

#[cfg(target_arch = "x86_64")]
use ostd::{arch::cpu::context::c_user_regs_struct, mm::VmIo};

use super::SyscallReturn;
use crate::{
    prelude::*,
    process::posix_thread::{
        AsPosixThread, alien_access::AlienAccessMode, ptrace::PtraceContRequest,
    },
    thread::{Thread, Tid},
};

pub fn sys_ptrace(
    request: u32,
    tid: Tid,
    addr: usize,
    data: usize,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let request = PtraceRequest::try_from(request)?;
    debug!(
        "ptrace: request = {:?}, tid = {}, addr = 0x{:x}, data = 0x{:x}",
        request, tid, addr, data
    );

    match request {
        PtraceRequest::PTRACE_TRACEME => {
            let current_thread = current_thread!();
            let parent_guard = ctx.process.parent().lock();
            let parent_main_thread = parent_guard.process().upgrade().unwrap().main_thread();

            do_ptrace_attach(parent_main_thread, current_thread)?;
        }
        #[cfg(target_arch = "x86_64")]
        PtraceRequest::PTRACE_PEEKUSER => {
            let tracee = ctx.posix_thread.get_tracee(tid)?;
            let tracee = tracee.as_posix_thread().unwrap();

            let val = tracee.ptrace_peek_user(addr)?;
            ctx.user_space().write_val(data, &val)?;
        }
        #[cfg(target_arch = "x86_64")]
        PtraceRequest::PTRACE_POKEUSER => {
            let tracee = ctx.posix_thread.get_tracee(tid)?;
            let tracee = tracee.as_posix_thread().unwrap();

            tracee.ptrace_poke_user(addr, data)?;
        }
        PtraceRequest::PTRACE_CONT => {
            if data != 0 {
                return_errno_with_message!(
                    Errno::EOPNOTSUPP,
                    "delivering signal via `PTRACE_CONT` is not supported currently"
                );
            }

            let tracee = ctx.posix_thread.get_tracee(tid)?;
            tracee
                .as_posix_thread()
                .unwrap()
                .ptrace_continue(PtraceContRequest::Continue)?;
        }
        #[cfg(target_arch = "x86_64")]
        PtraceRequest::PTRACE_GETREGS => {
            let tracee = ctx.posix_thread.get_tracee(tid)?;
            let tracee = tracee.as_posix_thread().unwrap();

            let regs = tracee.ptrace_get_regs()?;

            ctx.user_space().write_val(data, &regs)?;
        }
        #[cfg(target_arch = "x86_64")]
        PtraceRequest::PTRACE_SETREGS => {
            let regs = ctx.user_space().read_val::<c_user_regs_struct>(data)?;

            let tracee = ctx.posix_thread.get_tracee(tid)?;
            let tracee = tracee.as_posix_thread().unwrap();

            tracee.ptrace_set_regs(regs)?;
        }
        _ => {
            warn!("unimplemented ptrace request: {:?}", request);
            return_errno_with_message!(Errno::EOPNOTSUPP, "unimplemented ptrace request");
        }
    }

    Ok(SyscallReturn::Return(0))
}

fn do_ptrace_attach(tracer_thread: Arc<Thread>, tracee_thread: Arc<Thread>) -> Result<()> {
    let tracer = tracer_thread.as_posix_thread().unwrap();
    let tracee = tracee_thread.as_posix_thread().unwrap();
    if !Arc::ptr_eq(&tracer.process().main_thread(), &tracer_thread) {
        return_errno_with_message!(
            Errno::EOPNOTSUPP,
            "using a non-main thread as a tracer is not supported currently"
        );
    }

    if Weak::ptr_eq(tracer.weak_process(), tracee.weak_process()) {
        return_errno_with_message!(
            Errno::EPERM,
            "tracer and tracee must be in different processes"
        );
    }

    tracee.check_alien_access_from(tracer, AlienAccessMode::ATTACH_WITH_REAL_CREDS)?;

    tracee.set_tracer(Arc::downgrade(&tracer_thread))?;
    tracer.insert_tracee(tracee_thread);

    Ok(())
}

#[repr(u32)]
#[derive(Debug, Clone, Copy, TryFromInt)]
#[expect(non_camel_case_types)]
enum PtraceRequest {
    /// Indicate that this thread should be traced by its parent.
    PTRACE_TRACEME = 0,
    /// Return the word in the thread's text space at address ADDR.
    PTRACE_PEEKTEXT = 1,
    /// Return the word in the thread's data space at address ADDR.
    PTRACE_PEEKDATA = 2,
    /// Return the word in the thread's user area at offset ADDR.
    PTRACE_PEEKUSER = 3,
    /// Write the word DATA into the thread's text space at address ADDR.
    PTRACE_POKETEXT = 4,
    /// Write the word DATA into the thread's data space at address ADDR.
    PTRACE_POKEDATA = 5,
    /// Write the word DATA into the thread's user area at offset ADDR.
    PTRACE_POKEUSER = 6,
    /// Continue the thread.
    PTRACE_CONT = 7,
    /// Kill the thread.
    PTRACE_KILL = 8,
    /// Single step the thread.
    PTRACE_SINGLESTEP = 9,
    /// Get all general purpose registers used by a thread.
    PTRACE_GETREGS = 12,
    /// Set all general purpose registers used by a thread.
    PTRACE_SETREGS = 13,
    /// Get all floating point registers used by a thread.
    PTRACE_GETFPREGS = 14,
    /// Set all floating point registers used by a thread.
    PTRACE_SETFPREGS = 15,
    /// Attach to a thread that is already running.
    PTRACE_ATTACH = 16,
    /// Detach from a thread attached to.
    PTRACE_DETACH = 17,
    /// Get all extended floating point registers used by a thread.
    PTRACE_GETFPXREGS = 18,
    /// Set all extended floating point registers used by a thread.
    PTRACE_SETFPXREGS = 19,
    /// Continue and stop at the next entry to or return from syscall.
    PTRACE_SYSCALL = 24,
    /// Continue and stop at the next syscall, it will not be executed.
    PTRACE_SYSEMU = 31,
    /// Single step the thread, the next syscall will not be executed.
    PTRACE_SYSEMU_SINGLESTEP = 32,
    /// Set ptrace filter options.
    PTRACE_SETOPTIONS = 0x4200,
    /// Get the siginfo of the last ptrace-stop.
    PTRACE_GETSIGINFO = 0x4202,
    /// Get register content.
    PTRACE_GETREGSET = 0x4204,
    /// Set register content.
    PTRACE_SETREGSET = 0x4205,
}
