// SPDX-License-Identifier: MPL-2.0

use alloc::{collections::VecDeque, sync::Arc, vec::Vec};

use super::{LocalRunQueue, Scheduler, UpdateFlags};
use crate::{
    cpu::{num_cpus, this_cpu},
    sync::SpinLock,
    task::Task,
};

/// A simple FIFO (First-In-First-Out) task scheduler.
pub struct FifoScheduler<T: FifoSchedEntity> {
    idx: SpinLock<usize>,
    /// A thread-safe queue to hold tasks waiting to be executed.
    rq: Vec<SpinLock<FifoRunQueue<T>>>,
}

impl<T: FifoSchedEntity> FifoScheduler<T> {
    /// Creates a new instance of `FifoScheduler`.
    pub fn new() -> Self {
        let mut rq = Vec::new();
        for _ in 0..num_cpus() {
            rq.push(SpinLock::new(FifoRunQueue::new()));
        }
        Self {
            idx: SpinLock::new(0),
            rq,
        }
    }
}

impl<T: FifoSchedEntity + Send + Sync> Scheduler<T> for FifoScheduler<T> {
    fn enqueue(&self, runnable: Arc<T>, _flags: super::EnqueueFlags) {
        let mut idx = self.idx.lock_irq_disabled();
        self.rq[*idx].lock_irq_disabled().queue.push_back(runnable);

        *idx = if *idx == self.rq.len() - 1 {
            0
        } else {
            *idx + 1
        };
    }

    fn local_rq_with(&self, f: &mut dyn FnMut(&dyn LocalRunQueue<T>)) {
        let local_rq: &FifoRunQueue<T> = &self.rq[this_cpu() as usize].lock_irq_disabled();
        f(local_rq);
    }

    fn local_mut_rq_with(&self, f: &mut dyn FnMut(&mut dyn LocalRunQueue<T>)) {
        let local_rq: &mut FifoRunQueue<T> = &mut self.rq[this_cpu() as usize].lock_irq_disabled();
        f(local_rq);
    }
}

pub trait FifoSchedEntity {}

impl FifoSchedEntity for Task {}

struct FifoRunQueue<T: FifoSchedEntity> {
    current: Option<Arc<T>>,
    queue: VecDeque<Arc<T>>,
}

impl<T: FifoSchedEntity> FifoRunQueue<T> {
    pub const fn new() -> Self {
        Self {
            current: None,
            queue: VecDeque::new(),
        }
    }
}

impl<T: FifoSchedEntity> LocalRunQueue<T> for FifoRunQueue<T> {
    fn update_current(&mut self, flags: super::UpdateFlags) -> bool {
        !matches!(flags, UpdateFlags::Tick)
    }

    fn dequeue_current(&mut self) -> Option<Arc<T>> {
        self.current.take()
    }

    fn pick_next_current(&mut self) -> Option<Arc<T>> {
        self.queue.pop_front()
    }

    fn set_current(&mut self, next: Option<Arc<T>>) {
        let prev = self.current.take();
        self.current = next;
        if let Some(prev_task) = prev {
            self.queue.push_back(prev_task);
        }
    }

    fn current(&self) -> Option<&Arc<T>> {
        self.current.as_ref()
    }

    fn set_should_preempt(&mut self, _should_preempt: bool) {}

    fn should_preempt(&self) -> bool {
        false
    }
}

impl Default for FifoScheduler<Task> {
    fn default() -> Self {
        Self::new()
    }
}
