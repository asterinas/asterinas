// SPDX-License-Identifier: MPL-2.0

use alloc::collections::VecDeque;
use core::sync::atomic::{
    AtomicBool,
    Ordering::{self, Relaxed},
};

use crate::{
    cpu::{AtomicCpuSet, CpuId, CpuSet, PinCurrentCpu},
    prelude::*,
    sync::SpinLock,
    task::atomic_mode::AsAtomicModeGuard,
};

/// A RCU monitor ensures the completion of _grace periods_ by keeping track
/// of each CPU's passing _quiescent states_.
pub(super) struct RcuMonitor {
    is_monitoring: AtomicBool,
    state: SpinLock<State>,
}

impl RcuMonitor {
    /// Creates a new RCU monitor.
    ///
    /// This function is used to initialize a singleton instance of `RcuMonitor`.
    /// The singleton instance is globally accessible via the `RCU_MONITOR`.
    pub(super) fn new() -> Self {
        Self {
            is_monitoring: AtomicBool::new(false),
            state: SpinLock::new(State::new()),
        }
    }

    pub(super) unsafe fn finish_grace_period(&self) {
        // Fast path
        if !self.is_monitoring.load(Relaxed) {
            return;
        }

        // Check if the current GP is complete after passing the quiescent state
        // on the current CPU. If GP is complete, take the callbacks of the current
        // GP.
        let callbacks = {
            let mut state = self.state.disable_irq().lock();
            let cpu = state.as_atomic_mode_guard().current_cpu();
            if state.current_gp.is_complete() {
                return;
            }

            state.current_gp.finish_grace_period(cpu);
            if !state.current_gp.is_complete() {
                return;
            }

            // Now that the current GP is complete, take its callbacks
            let current_callbacks = state.current_gp.take_callbacks();

            // Check if we need to watch for a next GP
            if !state.next_callbacks.is_empty() {
                let callbacks = core::mem::take(&mut state.next_callbacks);
                state.current_gp.restart(callbacks);
            } else {
                self.is_monitoring.store(false, Relaxed);
            }

            current_callbacks
        };

        // Invoke the callbacks to notify the completion of GP
        for f in callbacks {
            (f)();
        }
    }

    pub(super) fn after_grace_period<F>(&self, f: F)
    where
        F: FnOnce() + Send + 'static,
    {
        let mut state = self.state.disable_irq().lock();

        state.next_callbacks.push_back(Box::new(f));

        if !state.current_gp.is_complete() {
            return;
        }

        let callbacks = core::mem::take(&mut state.next_callbacks);
        state.current_gp.restart(callbacks);
        self.is_monitoring.store(true, Relaxed);
    }
}

struct State {
    current_gp: GracePeriod,
    next_callbacks: Callbacks,
}

impl State {
    fn new() -> Self {
        Self {
            current_gp: GracePeriod::new(),
            next_callbacks: VecDeque::new(),
        }
    }
}

type Callbacks = VecDeque<Box<dyn FnOnce() + Send + 'static>>;

struct GracePeriod {
    callbacks: Callbacks,
    cpu_mask: AtomicCpuSet,
    is_complete: bool,
}

impl GracePeriod {
    fn new() -> Self {
        Self {
            callbacks: Callbacks::new(),
            cpu_mask: AtomicCpuSet::new(CpuSet::new_empty()),
            is_complete: true,
        }
    }

    fn is_complete(&self) -> bool {
        self.is_complete
    }

    fn finish_grace_period(&mut self, this_cpu: CpuId) {
        self.cpu_mask.add(this_cpu, Ordering::Relaxed);

        if self.cpu_mask.load(Ordering::Relaxed).is_full() {
            self.is_complete = true;
        }
    }

    fn take_callbacks(&mut self) -> Callbacks {
        core::mem::take(&mut self.callbacks)
    }

    fn restart(&mut self, callbacks: Callbacks) {
        self.is_complete = false;
        self.cpu_mask.store(&CpuSet::new_empty(), Ordering::Relaxed);
        self.callbacks = callbacks;
    }
}
