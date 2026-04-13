// SPDX-License-Identifier: MPL-2.0

use ostd::mm::VmIo;

use super::SyscallReturn;
#[cfg(target_arch = "x86_64")]
use crate::arch::ptrace as arch_ptrace;
use crate::{
    prelude::*,
    process::{
        posix_thread::{
            AsPosixThread,
            alien_access::AlienAccessMode,
            ptrace::{PtraceContRequest, PtraceOptions},
        },
        signal::{constants::SIGKILL, sig_num::SigNum, signals::user::UserSignal},
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
    let request = PtraceRequest::try_from(request)
        .map_err(|_| Error::with_message(Errno::EIO, "invalid ptrace request"))?;
    debug!(
        "ptrace: request = {:?}, tid = {}, addr = 0x{:x}, data = 0x{:x}",
        request, tid, addr, data
    );

    match request {
        PtraceRequest::PTRACE_TRACEME => {
            let current_thread = current_thread!();
            let parent_guard = ctx.process.parent().lock();
            let parent_main_thread = parent_guard.process().upgrade().unwrap().main_thread();

            do_ptrace_attach(&parent_main_thread, current_thread)?;
        }
        PtraceRequest::PTRACE_PEEKTEXT | PtraceRequest::PTRACE_PEEKDATA => {
            let tracee = ctx.posix_thread.get_tracee(tid)?;
            let tracee = tracee.as_posix_thread().unwrap();

            let val = tracee.ptrace_peek_data(addr)?;
            ctx.user_space().write_val(data, &val)?;
        }
        #[cfg(target_arch = "x86_64")]
        PtraceRequest::PTRACE_PEEKUSER => {
            let tracee = ctx.posix_thread.get_tracee(tid)?;
            let tracee = tracee.as_posix_thread().unwrap();

            let val = tracee.ptrace_peek_user(addr)?;
            ctx.user_space().write_val(data, &val)?;
        }
        PtraceRequest::PTRACE_POKETEXT | PtraceRequest::PTRACE_POKEDATA => {
            let tracee = ctx.posix_thread.get_tracee(tid)?;
            let tracee = tracee.as_posix_thread().unwrap();

            tracee.ptrace_poke_data(addr, data)?;
        }
        #[cfg(target_arch = "x86_64")]
        PtraceRequest::PTRACE_POKEUSER => {
            let tracee = ctx.posix_thread.get_tracee(tid)?;
            let tracee = tracee.as_posix_thread().unwrap();

            tracee.ptrace_poke_user(addr, data)?;
        }
        PtraceRequest::PTRACE_CONT => {
            let sig_num = parse_ptrace_injected_signal(data)?;

            let tracee = ctx.posix_thread.get_tracee(tid)?;
            let tracee = tracee.as_posix_thread().unwrap();

            tracee.ptrace_continue(PtraceContRequest::Continue(sig_num), ctx)?;
        }
        PtraceRequest::PTRACE_KILL => {
            let tracee = ctx.posix_thread.get_tracee(tid)?;
            let tracee = tracee.as_posix_thread().unwrap();

            tracee.enqueue_signal(Box::new(UserSignal::new_kill(SIGKILL, ctx)));
        }
        #[cfg(target_arch = "x86_64")]
        PtraceRequest::PTRACE_SINGLESTEP => {
            let sig_num = parse_ptrace_injected_signal(data)?;

            let tracee = ctx.posix_thread.get_tracee(tid)?;
            let tracee = tracee.as_posix_thread().unwrap();

            tracee.ptrace_continue(PtraceContRequest::SingleStep(sig_num), ctx)?;
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
            let tracee = ctx.posix_thread.get_tracee(tid)?;
            let tracee = tracee.as_posix_thread().unwrap();

            let regs = ctx
                .user_space()
                .read_val::<arch_ptrace::CUserRegsStruct>(data)?;
            tracee.ptrace_set_regs(regs)?;
        }
        PtraceRequest::PTRACE_SYSCALL => {
            let sig_num = parse_ptrace_injected_signal(data)?;

            let tracee = ctx.posix_thread.get_tracee(tid)?;
            let tracee = tracee.as_posix_thread().unwrap();

            tracee.ptrace_continue(PtraceContRequest::Syscall(sig_num), ctx)?;
        }
        PtraceRequest::PTRACE_SETOPTIONS => {
            let options = PtraceOptions::from_bits(data)
                .ok_or_else(|| Error::with_message(Errno::EINVAL, "invalid ptrace options"))?;

            let tracee = ctx.posix_thread.get_tracee(tid)?;
            let tracee = tracee.as_posix_thread().unwrap();

            tracee.ptrace_set_options(options)?;
        }
        PtraceRequest::PTRACE_GETEVENTMSG => {
            let tracee = ctx.posix_thread.get_tracee(tid)?;
            let tracee = tracee.as_posix_thread().unwrap();

            let event = tracee.ptrace_get_event()?;
            let eventmsg = event.map(|event| event.message()).unwrap_or(0);
            ctx.user_space().write_val(data, &eventmsg)?;
        }
        PtraceRequest::PTRACE_GETSIGINFO => {
            let tracee = ctx.posix_thread.get_tracee(tid)?;
            let siginfo = tracee.as_posix_thread().unwrap().ptrace_get_siginfo()?;

            ctx.user_space().write_val(data, &siginfo)?;
        }
    }

    Ok(SyscallReturn::Return(0))
}

fn do_ptrace_attach(tracer_thread: &Arc<Thread>, tracee_thread: Arc<Thread>) -> Result<()> {
    let tracer = tracer_thread.as_posix_thread().unwrap();
    let tracee = tracee_thread.as_posix_thread().unwrap();
    if !Arc::ptr_eq(&tracer.process().main_thread(), tracer_thread) {
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

    tracer.attach_to(tracer_thread, tracee_thread)
}

fn parse_ptrace_injected_signal(data: usize) -> Result<Option<SigNum>> {
    if data == 0 {
        return Ok(None);
    }

    let sig_num = u8::try_from(data)
        .map_err(|_| Error::with_message(Errno::EINVAL, "invalid signal number"))?;
    let sig_num = SigNum::try_from(sig_num)?;

    Ok(Some(sig_num))
}

#[repr(u32)]
#[derive(Debug, Clone, Copy, TryFromInt)]
#[expect(non_camel_case_types)]
enum PtraceRequest {
    /// Indicates that this thread should be traced by its parent.
    PTRACE_TRACEME = 0,
    /// Reads a word from the thread's text space at address `addr`.
    PTRACE_PEEKTEXT = 1,
    /// Reads a word from the thread's data space at address `addr`.
    PTRACE_PEEKDATA = 2,
    /// Reads a word from the thread's user area at offset `addr`.
    #[cfg(target_arch = "x86_64")]
    PTRACE_PEEKUSER = 3,
    /// Writes the word `data` to the thread's text space at address `addr`.
    PTRACE_POKETEXT = 4,
    /// Writes the word `data` to the thread's data space at address `addr`.
    PTRACE_POKEDATA = 5,
    /// Writes the word `data` to the thread's user area at offset `addr`.
    #[cfg(target_arch = "x86_64")]
    PTRACE_POKEUSER = 6,
    /// Continues the thread.
    PTRACE_CONT = 7,
    /// Kills the thread.
    PTRACE_KILL = 8,
    /// Single-steps the thread.
    #[cfg(target_arch = "x86_64")]
    PTRACE_SINGLESTEP = 9,
    /// Gets all general-purpose registers used by the thread.
    #[cfg(target_arch = "x86_64")]
    PTRACE_GETREGS = 12,
    /// Sets all general-purpose registers used by the thread.
    #[cfg(target_arch = "x86_64")]
    PTRACE_SETREGS = 13,
    /// Continues and stops at the next entry to or return from syscall.
    PTRACE_SYSCALL = 24,
    /// Sets ptrace options.
    PTRACE_SETOPTIONS = 0x4200,
    /// Gets the message of the last ptrace-event-stop.
    PTRACE_GETEVENTMSG = 0x4201,
    /// Gets the `siginfo` of the last ptrace-stop.
    PTRACE_GETSIGINFO = 0x4202,
    // TODO: Support other operations.
    // /// Gets all floating-point registers used by the thread.
    // PTRACE_GETFPREGS = 14,
    // /// Sets all floating-point registers used by the thread.
    // PTRACE_SETFPREGS = 15,
    // /// Attaches to a thread that is already running.
    // PTRACE_ATTACH = 16,
    // /// Detaches from an attached thread.
    // PTRACE_DETACH = 17,
    // /// Gets all extended floating-point registers used by the thread.
    // PTRACE_GETFPXREGS = 18,
    // /// Sets all extended floating-point registers used by the thread.
    // PTRACE_SETFPXREGS = 19,
    // /// Continues and stops at the next syscall, which will not be executed.
    // PTRACE_SYSEMU = 31,
    // /// Single-steps the thread, and the next syscall will not be executed.
    // PTRACE_SYSEMU_SINGLESTEP = 32,
    // /// Gets register contents.
    // PTRACE_GETREGSET = 0x4204,
    // /// Sets register contents.
    // PTRACE_SETREGSET = 0x4205,
}
