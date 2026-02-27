// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicBool, Ordering};

use hashbrown::HashMap;

use super::{AsPosixThread, PosixThread};
use crate::{
    prelude::*,
    process::signal::{c_types::siginfo_t, signals::Signal},
    thread::{Thread, Tid},
};

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
    /// - Ok(()) if the thread is traced and the ptrace-stop is triggered.
    /// - Err(signal) if the thread is not being traced, returning the original signal.
    pub(in crate::process) fn ptrace_stop(
        &self,
        signal: Box<dyn Signal>,
    ) -> core::result::Result<(), Box<dyn Signal>> {
        if let Some(status) = self.tracee_status.get() {
            status.ptrace_stop(signal)
        } else {
            Err(signal)
        }
    }

    /// Returns whether this thread is stopped by ptrace.
    pub fn is_ptrace_stopped(&self) -> bool {
        if let Some(status) = self.tracee_status.get() {
            status.is_ptrace_stopped()
        } else {
            false
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
        state.set_tracer(tracer);

        Ok(())
    }

    fn detach_tracer(&self) {
        // Hold the lock first to avoid race conditions.
        let mut tracee_state = self.state.lock();

        tracee_state.detach_tracer();
        tracee_state.siginfo = None;
        self.is_stopped.store(false, Ordering::Relaxed);
    }

    fn ptrace_stop(&self, signal: Box<dyn Signal>) -> core::result::Result<(), Box<dyn Signal>> {
        // Hold the lock first to avoid race conditions.
        let mut tracee_state = self.state.lock();

        let Some(tracer) = tracee_state.tracer() else {
            return Err(signal);
        };

        if !self.is_stopped.load(Ordering::Relaxed) {
            self.is_stopped.store(true, Ordering::Relaxed);
            tracee_state.siginfo = Some(signal.to_info());
            let tracer_process = tracer.as_posix_thread().unwrap().process();
            tracer_process.children_wait_queue().wake_all();
        }

        Ok(())
    }

    fn is_ptrace_stopped(&self) -> bool {
        self.is_stopped.load(Ordering::Relaxed)
    }
}

struct TraceeState {
    tracer: Weak<Thread>,
    /// The siginfo of the signal that stopped the tracee and has not yet been waited on.
    siginfo: Option<siginfo_t>,
}

impl TraceeState {
    fn new() -> Self {
        Self {
            tracer: Weak::new(),
            siginfo: None,
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
