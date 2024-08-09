// SPDX-License-Identifier: MPL-2.0

use alloc::collections::VecDeque;
use core::sync::atomic::{
    AtomicBool,
    Ordering::{Acquire, Relaxed, Release},
};

#[cfg(target_arch = "x86_64")]
use crate::arch::x86::cpu;
use crate::prelude::*;
use crate::sync::AtomicBits;
use crate::sync::SpinLock;

/// A RCU monitor ensures the completion of _grace periods_ by keeping track
/// of each CPU's passing _quiescent states_.
pub struct RcuMonitor {
    is_monitoring: AtomicBool,
    state: SpinLock<State>,
}

impl RcuMonitor {
    pub fn new(num_cpus: usize) -> Self {
        Self {
            is_monitoring: AtomicBool::new(false),
            state: SpinLock::new(State::new(num_cpus)),
        }
    }

    pub unsafe fn pass_quiescent_state(&self) {
        // Fast path
        if !self.is_monitoring.load(Relaxed) {
            return;
        }

        // Check if the current GP is complete after passing the quiescent state
        // on the current CPU. If GP is complete, take the callbacks of the current
        // GP.
        let callbacks = {
            let mut state = self.state.disable_irq().lock();
            if state.current_gp.is_complete() {
                return;
            }

            state.current_gp.pass_quiescent_state();
            if !state.current_gp.is_complete() {
                return;
            }

            // Now that the current GP is complete, take its callbacks
            let current_callbacks = state.current_gp.take_callbacks();

            // Check if we need to70G watch for a next GP
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

    pub fn after_grace_period<F>(&self, f: F)
    where
        F: FnOnce() -> () + Send + 'static,
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
    pub fn new(num_cpus: usize) -> Self {
        Self {
            current_gp: GracePeriod::new(num_cpus),
            next_callbacks: VecDeque::new(),
        }
    }
}

type Callbacks = VecDeque<Box<dyn FnOnce() -> () + Send + 'static>>;

struct GracePeriod {
    callbacks: Callbacks,
    cpu_mask: AtomicBits,
    is_complete: bool,
}

impl GracePeriod {
    pub fn new(num_cpus: usize) -> Self {
        Self {
            callbacks: Default::default(),
            cpu_mask: AtomicBits::new_zeroes(num_cpus),
            is_complete: false,
        }
    }

    pub fn is_complete(&self) -> bool {
        self.is_complete
    }

    pub unsafe fn pass_quiescent_state(&mut self) {
        let this_cpu = cpu::this_cpu();
        self.cpu_mask.set(this_cpu as usize, true);

        if self.cpu_mask.is_full() {
            self.is_complete = true;
        }
    }

    pub fn take_callbacks(&mut self) -> Callbacks {
        core::mem::take(&mut self.callbacks)
    }

    pub fn restart(&mut self, callbacks: Callbacks) {
        self.is_complete = false;
        self.cpu_mask.clear();
        self.callbacks = callbacks;
    }
}
