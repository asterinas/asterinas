// SPDX-License-Identifier: MPL-2.0

//! Ptrace implementation for POSIX threads.

use core::sync::atomic::{AtomicBool, Ordering};

use ostd::sync::Waiter;

use super::{AsPosixThread, PosixThread};
use crate::{
    prelude::*,
    process::signal::{
        DequeuedSignal, PauseReason,
        c_types::siginfo_t,
        constants::{CLD_TRAPPED, SIGCHLD},
        signals::raw::RawSignal,
    },
    thread::{Thread, Tid},
};

mod util;

pub(in crate::process) use util::PtraceStopResult;
use util::StopDeliverySignal;

impl PosixThread {
    /// Returns whether this thread is being traced.
    pub(in crate::process) fn is_traced(&self) -> bool {
        self.tracer().is_some()
    }

    /// Returns the tracer of this thread if it is being traced.
    pub fn tracer(&self) -> Option<Arc<Thread>> {
        self.tracee_status.get().and_then(|status| status.tracer())
    }

    /// Sets the tracer of this thread.
    ///
    /// # Errors
    ///
    /// Returns `EPERM` if this thread is already being traced.
    fn set_tracer(&self, tracer: Weak<Thread>) -> Result<()> {
        let status = self.tracee_status.call_once(TraceeStatus::new);
        status.set_tracer(tracer)
    }

    /// Detaches the tracer of this thread.
    pub(in crate::process) fn detach_tracer(&self) {
        if let Some(status) = self.tracee_status.get() {
            status.detach_tracer();
            self.wake_signalled_waker();
        }
    }

    /// Stops this thread by ptrace with the given signal if it is currently traced.
    ///
    /// Returns a [`PtraceStopResult`] indicating why this ptrace-stop ended.
    pub(in crate::process) fn ptrace_stop(
        &self,
        signal: DequeuedSignal,
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
    /// Attaches this tracer to the given tracee.
    ///
    /// # Errors
    ///
    /// Returns `EPERM` if the tracee is already being traced.
    ///
    /// # Panics
    ///
    /// Panics if `tracer_thread` and `self` do not point to the same thread,
    /// or if `tracee_thread` is not a POSIX thread.
    pub fn attach_to(&self, tracer_thread: &Arc<Thread>, tracee_thread: Arc<Thread>) -> Result<()> {
        debug_assert!(core::ptr::eq(
            tracer_thread.as_posix_thread().unwrap(),
            self
        ));

        let tracees = self.tracees.call_once(|| Mutex::new(BTreeMap::new()));

        // Lock order: tracer.tracees -> tracee.tracee_status
        let mut tracees = tracees.lock();
        if tracer_thread.is_exited() {
            // Pretend that the tracer immediately detaches the tracee,
            // if the tracer has already exited.
            // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/kernel/ptrace.c#L496-L498>
            return Ok(());
        }

        let tracee = tracee_thread.as_posix_thread().unwrap();
        tracee.set_tracer(Arc::downgrade(tracer_thread))?;
        tracees.insert(tracee.tid(), tracee_thread);

        Ok(())
    }

    /// Returns the tracee map of this thread if it is a tracer.
    pub(in crate::process) fn tracees(&self) -> Option<&Mutex<BTreeMap<Tid, Arc<Thread>>>> {
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
        self.is_stopped.store(false, Ordering::Relaxed);
    }

    fn ptrace_stop(&self, signal: DequeuedSignal, ctx: &Context) -> PtraceStopResult {
        // Hold the lock first to avoid race conditions.
        let mut state = self.state.lock();

        let Some(tracer) = state.tracer() else {
            return PtraceStopResult::NotTraced(signal);
        };

        debug_assert!(!self.is_ptrace_stopped());

        state.signal.stop(signal);
        self.is_stopped.store(true, Ordering::Relaxed);
        drop(state);

        let tracer = tracer.as_posix_thread().unwrap();
        tracer.enqueue_signal(Box::new(RawSignal::new({
            let mut siginfo = siginfo_t::new(SIGCHLD, CLD_TRAPPED);
            siginfo.set_pid_uid_by(ctx);
            siginfo
        })));
        tracer.process().children_wait_queue().wake_all();

        let waiter = Waiter::new_pair().0;
        if waiter
            .pause_until_by(
                || (!self.is_ptrace_stopped()).then_some(()),
                PauseReason::StopByPtrace,
            )
            .is_err()
        {
            // A `SIGKILL` interrupts this ptrace-stop.
            let mut state = self.state.lock();
            state.signal.clear();
            self.is_stopped.store(false, Ordering::Relaxed);
            return PtraceStopResult::Interrupted;
        };

        let mut state = self.state.lock();
        let signal = state.signal.clear();
        PtraceStopResult::Continued(signal)
    }

    fn is_ptrace_stopped(&self) -> bool {
        self.is_stopped.load(Ordering::Relaxed)
    }
}

struct TraceeState {
    tracer: Weak<Thread>,
    /// The signal associated with the current ptrace-stop and later signal delivery.
    signal: StopDeliverySignal,
}

impl TraceeState {
    fn new() -> Self {
        Self {
            tracer: Weak::new(),
            signal: StopDeliverySignal::default(),
        }
    }

    fn tracer(&self) -> Option<Arc<Thread>> {
        self.tracer.upgrade()
    }
}
