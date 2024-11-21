// SPDX-License-Identifier: MPL-2.0

use alloc::collections::VecDeque;
use core::sync::atomic::{AtomicBool, Ordering};

use crate::{
    cpu::{AtomicCpuSet, CpuSet, PinCurrentCpu},
    prelude::*,
    sync::SpinLock,
    task::disable_preempt,
};

/// A RCU monitor ensures the completion of _grace periods_ by keeping track
/// of each CPU's passing _quiescent states_.
pub struct RcuMonitor {
    cur_queue_dropping: AtomicBool,
    cpus_passed_quiescent: AtomicCpuSet,
    cur_waitqueue: SpinLock<Callbacks>,
    next_waitqueue: SpinLock<Callbacks>,
}

impl RcuMonitor {
    /// Creates a new RCU monitor.
    ///
    /// This function is used to initialize a singleton instance of `RcuMonitor`.
    /// The singleton instance is globally accessible via the `RCU_MONITOR`.
    pub fn new() -> Self {
        Self {
            cur_queue_dropping: AtomicBool::new(false),
            cpus_passed_quiescent: AtomicCpuSet::new(CpuSet::new_empty()),
            cur_waitqueue: SpinLock::new(VecDeque::new()),
            next_waitqueue: SpinLock::new(VecDeque::new()),
        }
    }

    pub(super) unsafe fn pass_quiescent_state(&self) {
        let preempt_guard = disable_preempt();
        let current_cpu = preempt_guard.current_cpu();

        self.cpus_passed_quiescent
            .add(current_cpu, Ordering::Relaxed);

        if self.cpus_passed_quiescent.load().is_full() {
            if self
                .cur_queue_dropping
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
            {
                return;
            };

            let mut to_be_drained = VecDeque::new();

            let mut cur_waitqueue = self.cur_waitqueue.lock();
            core::mem::swap(&mut *cur_waitqueue, &mut to_be_drained);
            let mut next_waitqueue = self.next_waitqueue.lock();
            core::mem::swap(&mut *cur_waitqueue, &mut *next_waitqueue);
            drop(next_waitqueue);
            drop(cur_waitqueue);

            self.cpus_passed_quiescent.store(&CpuSet::new_empty());
            self.cur_queue_dropping.store(false, Ordering::Release);

            for callback in to_be_drained.drain(..) {
                callback();
            }
        }
    }

    pub fn after_grace_period<F>(&self, f: F)
    where
        F: FnOnce() + Send + 'static,
    {
        let mut next_waitqueue = self.next_waitqueue.lock();
        next_waitqueue.push_back(Box::new(f));
    }
}

type Callbacks = VecDeque<Box<dyn FnOnce() + Send + 'static>>;
