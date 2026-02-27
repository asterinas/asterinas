// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicBool, Ordering};

use bitflags::bitflags;
use inherit_methods_macro::inherit_methods;

use crate::{
    prelude::*,
    process::{
        credentials::capabilities::CapSet,
        posix_thread::{AsPosixThread, PosixThread},
        signal::{c_types::siginfo_t, sig_num::SigNum, signals::Signal},
    },
    thread::Thread,
};

pub(super) struct TraceeStatus {
    is_stopped: AtomicBool,
    state: Mutex<TraceeState>,
}

#[inherit_methods(from = "self.state.lock()")]
impl TraceeStatus {
    pub(super) fn tracer(&self) -> Option<Arc<Thread>>;
    pub(super) fn set_tracer(&self, tracer: Weak<Thread>);
    pub(super) fn detach_tracer(&self);

    pub(super) fn new() -> Self {
        Self {
            is_stopped: AtomicBool::new(false),
            state: Mutex::new(TraceeState::new()),
        }
    }

    pub(super) fn ptrace_stop(&self, signal: Box<dyn Signal>) {
        // Hold the lock first to avoid race conditions
        let mut tracee_state = self.state.lock();

        let Some(tracer) = tracee_state.tracer() else {
            return;
        };

        if !self.is_stopped.load(Ordering::Relaxed) {
            self.is_stopped.store(true, Ordering::Relaxed);
            tracee_state.siginfo = Some(signal.to_info());
            let tracer_process = tracer.as_posix_thread().unwrap().process();
            tracer_process.children_wait_queue().wake_all();
        }
    }

    pub(super) fn is_ptrace_stopped(&self) -> bool {
        self.is_stopped.load(Ordering::Relaxed)
    }

    pub(super) fn wait(&self) -> Option<SigNum> {
        // Hold the lock first to avoid race conditions
        let mut tracee_state = self.state.lock();

        if let Some(siginfo) = tracee_state.siginfo.take() {
            let sig_num = (siginfo.si_signo as u8).try_into().unwrap();
            tracee_state.waited_siginfo = Some(siginfo);
            Some(sig_num)
        } else {
            None
        }
    }
}

struct TraceeState {
    tracer: Weak<Thread>,
    /// The siginfo of the signal that stopped the tracee and has not yet been waited on.
    siginfo: Option<siginfo_t>,
    /// The siginfo of the signal that stopped the tracee and has already been waited on.
    ///
    /// This is needed to support `PTRACE_GETSIGINFO`.
    waited_siginfo: Option<siginfo_t>,
}

impl TraceeState {
    fn new() -> Self {
        Self {
            tracer: Weak::new(),
            siginfo: None,
            waited_siginfo: None,
        }
    }

    fn tracer(&self) -> Option<Arc<Thread>> {
        self.tracer.upgrade()
    }

    fn set_tracer(&mut self, tracer: Weak<Thread>) {
        self.tracer = tracer;
    }

    fn detach_tracer(&mut self) {
        self.tracer = Weak::new();
    }
}

/// Checks whether the current `PosixThread` may access the given target
/// `PosixThread` via ptrace operations.
// Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/kernel/ptrace.c#L276>
pub fn check_may_access(
    current_pthread: &PosixThread,
    target_pthread: &PosixThread,
    mode: PtraceMode,
) -> Result<()> {
    if Weak::ptr_eq(
        current_pthread.weak_process(),
        target_pthread.weak_process(),
    ) {
        return Ok(());
    }

    let cred = current_pthread.credentials();
    let (caller_uid, caller_gid) = if mode.1 == PtraceCredsMode::FsCreds {
        (cred.fsuid(), cred.fsgid())
    } else {
        (cred.ruid(), cred.rgid())
    };

    let tcred = target_pthread.credentials();
    let caller_is_same = caller_uid == tcred.euid()
        && caller_uid == tcred.suid()
        && caller_uid == tcred.ruid()
        && caller_gid == tcred.egid()
        && caller_gid == tcred.sgid()
        && caller_gid == tcred.rgid();
    let caller_has_cap = target_pthread
        .process()
        .user_ns()
        .lock()
        .check_cap(CapSet::SYS_PTRACE, current_pthread)
        .is_ok();

    if !caller_is_same && !caller_has_cap {
        return_errno_with_message!(
            Errno::EPERM,
            "the calling process does not have the required permissions"
        );
    }

    // TODO: Add further security checks (e.g., YAMA LSM).

    Ok(())
}

#[expect(dead_code)]
pub struct PtraceMode(PtraceFlags, PtraceCredsMode);

impl PtraceMode {
    #[expect(dead_code)]
    pub const READ_REALCREDS: Self = Self(PtraceFlags::READ, PtraceCredsMode::RealCreds);
    pub const ATTACH_REALCREDS: Self = Self(PtraceFlags::ATTACH, PtraceCredsMode::RealCreds);
    pub const READ_FSCREDS: Self = Self(PtraceFlags::READ, PtraceCredsMode::FsCreds);
    pub const ATTACH_FSCREDS: Self = Self(PtraceFlags::ATTACH, PtraceCredsMode::FsCreds);
}

bitflags! {
    struct PtraceFlags: u32 {
        const READ       = 0x01;
        const ATTACH     = 0x02;
        const NOAUDIT    = 0x04;
    }
}

#[derive(PartialEq)]
enum PtraceCredsMode {
    FsCreds,
    RealCreds,
}
