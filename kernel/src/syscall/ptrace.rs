// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    prelude::*,
    process::{
        posix_thread::{thread_table, AsPosixThread, TraceeStatus},
        signal::{
            sig_num::SigNum,
            signals::user::{UserSignal, UserSignalKind},
        },
        tgkill,
    },
    thread::{Thread, Tid},
};

pub fn sys_ptrace(
    request: u32,
    pid: Tid,
    addr: u64,
    data: u64,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let request = PtraceRequest::try_from(request)?;
    debug!(
        "request = {:?}, pid = {}, addr = 0x{:x}, data = 0x{:x}",
        request, pid, addr, data
    );

    match request {
        PtraceRequest::PTRACE_TRACEME => {
            let current_posix_thread = ctx.posix_thread;
            let current_thread = thread_table::get_thread(current_posix_thread.tid()).unwrap();

            let parent_guard = ctx.process.parent().lock();
            let tracer_thread = parent_guard.process().upgrade().unwrap().main_thread();
            drop(parent_guard);

            do_ptrace_attach(tracer_thread, current_thread)?;
        }
        PtraceRequest::PTRACE_CONT => {
            let tracees = ctx.posix_thread.tracees().lock();
            let Some(tracee) = tracees.get(&pid) else {
                return_errno_with_message!(Errno::ESRCH, "No such tracee");
            };
            let tracee_process = tracee.as_posix_thread().unwrap().process();
            tracee_process.resume();
            if data != 0 {
                let signal = {
                    let pid = ctx.process.pid();
                    let uid = ctx.posix_thread.credentials().ruid();
                    UserSignal::new(
                        SigNum::try_from(data as u8)?,
                        UserSignalKind::Tkill,
                        pid,
                        uid,
                    )
                };
                tgkill(pid, tracee_process.pid(), Some(signal), ctx)?;
            }
        }
        _ => {
            warn!("Unimplemented ptrace request: {:?}", request);
            return_errno_with_message!(Errno::EOPNOTSUPP, "Unimplemented ptrace request");
        }
    };

    Ok(SyscallReturn::Return(0))
}

fn do_ptrace_attach(tracer_thread: Arc<Thread>, tracee_thread: Arc<Thread>) -> Result<()> {
    let tracer = tracer_thread
        .as_posix_thread()
        .ok_or(Error::new(Errno::EPERM))?;
    let tracee = tracee_thread
        .as_posix_thread()
        .ok_or(Error::new(Errno::EPERM))?;

    if Weak::ptr_eq(tracer.weak_process(), tracee.weak_process()) {
        return_errno_with_message!(
            Errno::EPERM,
            "Tracer and tracee must be in different processes"
        );
    }

    let mut tracee_status = tracee.tracee_status().lock();

    if let Some(old_tracer) = tracee_status
        .as_ref()
        .and_then(|status| status.tracer().upgrade())
    {
        let old_tracer = old_tracer.as_posix_thread().unwrap();
        if !Weak::ptr_eq(tracer.weak_process(), old_tracer.weak_process()) {
            return_errno_with_message!(Errno::EPERM, "Tracee is already being traced");
        }
    }

    tracer
        .tracees()
        .lock()
        .insert(tracee.tid(), tracee_thread.clone());
    tracee_status.replace(TraceeStatus::new(
        Arc::downgrade(&tracer_thread),
        tracer.tid(),
    ));

    // FIXME: Figure out Linux's behavior when a ptrace attach happens,
    // and then implement the corresponding logic.

    let tracee_process = tracee.process();
    let parent_process_guard = tracee_process.parent().lock();
    if let Some(parent) = parent_process_guard.process().upgrade() {
        let mut children = parent.children().lock();
        children.remove(&tracee_process.pid());
    }

    Ok(())
}

#[repr(u32)]
#[derive(Debug, Clone, Copy, TryFromInt)]
#[expect(non_camel_case_types)]
enum PtraceRequest {
    /// Indicate that the process making this request should be traced.
    /// All signals received by this process can be intercepted by its parent,
    /// and its parent can use the other ptrace requests.
    PTRACE_TRACEME = 0,
    /// Return the word in the process's text space at address ADDR.
    PTRACE_PEEKTEXT = 1,
    /// Return the word in the process's data space at address ADDR.
    PTRACE_PEEKDATA = 2,
    /// Return the word in the process's user area at offset ADDR.
    PTRACE_PEEKUSER = 3,
    /// Write the word DATA into the process's text space at address ADDR.
    PTRACE_POKETEXT = 4,
    /// Write the word DATA into the process's data space at address ADDR.
    PTRACE_POKEDATA = 5,
    /// Write the word DATA into the process's user area at offset ADDR.
    PTRACE_POKEUSER = 6,
    /// Continue the process.
    PTRACE_CONT = 7,
    /// Kill the process.
    PTRACE_KILL = 8,
    /// Single step the process.
    PTRACE_SINGLESTEP = 9,
    /// Get all general purpose registers used by a process.
    PTRACE_GETREGS = 12,
    /// Set all general purpose registers used by a process.
    PTRACE_SETREGS = 13,
    /// Get all floating point registers used by a process.
    PTRACE_GETFPREGS = 14,
    /// Set all floating point registers used by a process.
    PTRACE_SETFPREGS = 15,
    /// Attach to a process that is already running.
    PTRACE_ATTACH = 16,
    /// Detach from a process attached to with PTRACE_ATTACH.
    PTRACE_DETACH = 17,
    /// Get all extended floating point registers used by a process.
    PTRACE_GETFPXREGS = 18,
    /// Set all extended floating point registers used by a process.
    PTRACE_SETFPXREGS = 19,
    /// Continue and stop at the next entry to or return from syscall.
    PTRACE_SYSCALL = 24,
    /// Get a TLS entry in the GDT.
    PTRACE_GET_THREAD_AREA = 25,
    /// Change a TLS entry in the GDT.
    PTRACE_SET_THREAD_AREA = 26,
    #[cfg(target_arch = "x86_64")]
    /// Access TLS data (x86_64 only).
    PTRACE_ARCH_PRCTL = 30,
    /// Continue and stop at the next syscall, it will not be executed.
    PTRACE_SYSEMU = 31,
    /// Single step the process, the next syscall will not be executed.
    PTRACE_SYSEMU_SINGLESTEP = 32,
    /// Execute process until next taken branch.
    PTRACE_SINGLEBLOCK = 33,
    /// Set ptrace filter options.
    PTRACE_SETOPTIONS = 0x4200,
    /// Get last ptrace message.
    PTRACE_GETEVENTMSG = 0x4201,
    /// Get siginfo for process.
    PTRACE_GETSIGINFO = 0x4202,
    /// Set new siginfo for process.
    PTRACE_SETSIGINFO = 0x4203,
    /// Get register content.
    PTRACE_GETREGSET = 0x4204,
    /// Set register content.
    PTRACE_SETREGSET = 0x4205,
    /// Like PTRACE_ATTACH, but do not force tracee to trap and do not affect
    /// signal or group stop state.
    PTRACE_SEIZE = 0x4206,
    /// Trap seized tracee.
    PTRACE_INTERRUPT = 0x4207,
    /// Wait for next group event.
    PTRACE_LISTEN = 0x4208,
    /// Retrieve siginfo_t structures without removing signals from a queue.
    PTRACE_PEEKSIGINFO = 0x4209,
    /// Get the mask of blocked signals.
    PTRACE_GETSIGMASK = 0x420A,
    /// Change the mask of blocked signals.
    PTRACE_SETSIGMASK = 0x420B,
    /// Get seccomp BPF filters.
    PTRACE_SECCOMP_GET_FILTER = 0x420C,
    /// Get seccomp BPF filter metadata.
    PTRACE_SECCOMP_GET_METADATA = 0x420D,
    /// Get information about system call.
    PTRACE_GET_SYSCALL_INFO = 0x420E,
    /// Get rseq configuration information.
    PTRACE_GET_RSEQ_CONFIGURATION = 0x420F,
}
