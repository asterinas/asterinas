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
    /// A thread-safe queue to hold tasks waiting to be executed.
    rq: Vec<SpinLock<FifoRunQueue<T>>>,
}

impl<T: FifoSchedEntity> FifoScheduler<T> {
    /// Creates a new instance of `FifoScheduler`.
    fn new(ncpu: u32) -> Self {
        let mut rq = Vec::new();
        for _ in 0..ncpu {
            rq.push(SpinLock::new(FifoRunQueue::new()));
        }
        Self { rq }
    }

    fn select_cpu(&self) -> usize {
        // FIXME: adopt more reasonable policy once we fully enable SMP.
        0
    }
}

impl<T: FifoSchedEntity + Send + Sync> Scheduler<T> for FifoScheduler<T> {
    fn enqueue(&self, runnable: Arc<T>, flags: super::EnqueueFlags) -> Option<u32> {
        if flags == super::EnqueueFlags::Wake {
            for cpu_id in 0..self.rq.len() {
                let rq = self.rq[cpu_id].lock_irq_disabled();
                if rq.current.is_none() {
                    continue;
                }
                if Arc::ptr_eq(&runnable, rq.current.as_ref().unwrap()) {
                    return Some(cpu_id as u32);
                }
            }
        }
        let target_cpu = self.select_cpu();
        let mut rq = self.rq[target_cpu].lock_irq_disabled();
        rq.queue.push_back(runnable);

        Some(target_cpu as u32)
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
    fn current(&self) -> Option<&Arc<T>> {
        self.current.as_ref()
    }

    fn update_current(&mut self, flags: super::UpdateFlags) -> bool {
        !matches!(flags, UpdateFlags::Tick)
    }

    fn pick_next_current(&mut self, keep_current: bool) -> Option<Arc<T>> {
        let prev = self.current.take();
        let next = self.queue.pop_front();
        self.current = next.clone();
        if self.current.is_none() {
            self.current = prev;
        } else if keep_current && let Some(prev_task) = prev {
            self.queue.push_back(prev_task);
        }

        next
    }
}

impl Default for FifoScheduler<Task> {
    fn default() -> Self {
        Self::new(num_cpus())
    }
}
