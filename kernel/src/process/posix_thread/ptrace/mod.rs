// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicBool, Ordering};

use hashbrown::HashMap;
use ostd::sync::Waiter;

use super::{AsPosixThread, PosixThread};
use crate::{
    prelude::*,
    process::signal::{
        PauseReason,
        c_types::siginfo_t,
        constants::SIGCHLD,
        signals::{Signal, user::UserSignal},
    },
    thread::{Thread, Tid},
};

mod util;

pub(in crate::process) use util::PtraceStopResult;
use util::StopSigInfo;

impl PosixThread {
    /// Returns whether this thread may be a tracee.
    pub(in crate::process) fn may_be_tracee(&self) -> bool {
        self.tracee_status.is_completed()
    }

    /// Returns the tracer of this thread if it is being traced.
    pub fn tracer(&self) -> Option<Arc<Thread>> {
        self.tracee_status.get().and_then(|status| status.tracer())
    }

    /// Sets the tracer of this thread.
    ///
    /// # Errors
    ///
    /// Returns `EPERM` if the this thread is already being traced.
    pub fn set_tracer(&self, tracer: Weak<Thread>) -> Result<()> {
        let status = self.tracee_status.call_once(TraceeStatus::new);
        status.set_tracer(tracer)
    }

    /// Detaches the tracer of this thread.
    pub fn detach_tracer(&self) {
        if let Some(status) = self.tracee_status.get() {
            status.detach_tracer();
            self.wake_signalled_waker();
        }
    }

    /// Stops this thread by ptrace with the given signal if it is currently traced.
    ///
    /// Returns:
    /// - `PtraceStopResult::Continued` if the ptrace-stop is continued by the tracer.
    /// - `PtraceStopResult::Interrupted` if the ptrace-stop is interrupted by `SIGKILL`.
    /// - `PtraceStopResult::NotTraced(signal)` if the thread is not traced,
    ///   with the given `signal` returned back.
    pub(in crate::process) fn ptrace_stop(
        &self,
        signal: Box<dyn Signal>,
        ctx: &Context,
    ) -> PtraceStopResult {
        if let Some(status) = self.tracee_status.get() {
            status.ptrace_stop(signal, ctx)
        } else {
            PtraceStopResult::NotTraced(signal)
        }
    }
}

impl PosixThread {
    /// Inserts a tracee to the tracee map of this thread.
    ///
    /// # Panics
    ///
    /// This method will panic if the tracee is not a POSIX thread.
    pub fn insert_tracee(&self, tracee: Arc<Thread>) {
        let tracees = self.tracees.call_once(|| Mutex::new(HashMap::new()));
        tracees
            .lock()
            .insert(tracee.as_posix_thread().unwrap().tid(), tracee);
    }

    /// Removes the tracee with the given tid from the tracee map of this thread.
    #[expect(dead_code)]
    pub fn remove_tracee(&self, tid: Tid) {
        if let Some(tracees) = self.tracees.get() {
            tracees.lock().remove(&tid);
        }
    }

    /// Returns the tracee map of this thread if it is a tracer.
    pub(super) fn tracees(&self) -> Option<&Mutex<HashMap<Tid, Arc<Thread>>>> {
        self.tracees.get()
    }

    /// Clears all tracees of this tracer on exit.
    pub(in crate::process) fn clear_tracees(&self) {
        let Some(tracees) = self.tracees() else {
            return;
        };

        // Lock order: tracer.tracees -> tracee.tracee_status
        let tracees = tracees.lock();
        for (_, tracee) in tracees.iter() {
            let tracee = tracee.as_posix_thread().unwrap();
            tracee.detach_tracer();
        }
    }
}

pub(super) struct TraceeStatus {
    is_stopped: AtomicBool,
    state: Mutex<TraceeState>,
}

impl TraceeStatus {
    pub(super) fn new() -> Self {
        Self {
            is_stopped: AtomicBool::new(false),
            state: Mutex::new(TraceeState::new()),
        }
    }

    fn tracer(&self) -> Option<Arc<Thread>> {
        self.state.lock().tracer()
    }

    fn set_tracer(&self, tracer: Weak<Thread>) -> Result<()> {
        let mut state = self.state.lock();
        if state.tracer().is_some() {
            return_errno_with_message!(Errno::EPERM, "the thread is already being traced");
        }
        state.tracer = tracer;

        Ok(())
    }

    fn detach_tracer(&self) {
        // Hold the lock first to avoid race conditions.
        let mut state = self.state.lock();

        state.tracer = Weak::new();
        state.siginfo.clear();
        self.is_stopped.store(false, Ordering::Relaxed);
    }

    fn ptrace_stop(&self, signal: Box<dyn Signal>, ctx: &Context) -> PtraceStopResult {
        // Hold the lock first to avoid race conditions.
        let mut state = self.state.lock();

        let Some(tracer) = state.tracer() else {
            return PtraceStopResult::NotTraced(signal);
        };

        debug_assert!(!self.is_ptrace_stopped());

        state.siginfo.stop(signal.to_info());
        self.is_stopped.store(true, Ordering::Relaxed);

        let tracer = tracer.as_posix_thread().unwrap();
        tracer.enqueue_signal(Box::new(UserSignal::new_kill(SIGCHLD, ctx)));
        tracer.process().children_wait_queue().wake_all();

        drop(state);

        let waiter = Waiter::new_pair().0;
        if waiter
            .pause_until_by(
                || (!self.is_ptrace_stopped()).then_some(()),
                PauseReason::StopByPtrace,
            )
            .is_err()
        {
            // A `SIGKILL` interrupts this ptrace-stop.
            return PtraceStopResult::Interrupted;
        };

        PtraceStopResult::Continued
    }

    fn is_ptrace_stopped(&self) -> bool {
        self.is_stopped.load(Ordering::Relaxed)
    }
}

struct TraceeState {
    tracer: Weak<Thread>,
    /// The siginfo of the signal that stopped the tracee.
    siginfo: StopSigInfo,
}

impl TraceeState {
    fn new() -> Self {
        Self {
            tracer: Weak::new(),
            siginfo: StopSigInfo::default(),
        }
    }

    fn tracer(&self) -> Option<Arc<Thread>> {
        self.tracer.upgrade()
    }
}
