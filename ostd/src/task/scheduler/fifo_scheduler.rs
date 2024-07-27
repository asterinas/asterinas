// SPDX-License-Identifier: MPL-2.0

use alloc::{boxed::Box, collections::VecDeque, sync::Arc, vec::Vec};

use super::{inject_scheduler, EnqueueFlags, LocalRunQueue, Scheduler, UpdateFlags};
use crate::{
    cpu::{num_cpus, this_cpu},
    sync::SpinLock,
    task::{AtomicCpuId, Task},
};

pub fn init() {
    let fifo_scheduler = Box::new(FifoScheduler::default());
    let scheduler = Box::<FifoScheduler<Task>>::leak(fifo_scheduler);
    inject_scheduler(scheduler);
}

/// A simple FIFO (First-In-First-Out) task scheduler.
struct FifoScheduler<T: FifoSchedInfo> {
    /// A thread-safe queue to hold tasks waiting to be executed.
    rq: Vec<SpinLock<FifoRunQueue<T>>>,
}

impl<T: FifoSchedInfo> FifoScheduler<T> {
    /// Creates a new instance of `FifoScheduler`.
    fn new(nr_cpus: u32) -> Self {
        let mut rq = Vec::new();
        for _ in 0..nr_cpus {
            rq.push(SpinLock::new(FifoRunQueue::new()));
        }
        Self { rq }
    }

    fn select_cpu(&self) -> u32 {
        // FIXME: adopt more reasonable policy once we fully enable SMP.
        0
    }
}

impl<T: FifoSchedInfo + Send + Sync> Scheduler<T> for FifoScheduler<T> {
    fn enqueue(&self, runnable: Arc<T>, flags: EnqueueFlags) -> Option<u32> {
        let mut still_in_rq = false;
        let target_cpu = {
            let mut cpu_id = self.select_cpu();
            if let Err(task_cpu_id) = runnable.cpu().set_if_is_none(cpu_id) {
                debug_assert!(flags != EnqueueFlags::Spawn);
                still_in_rq = true;
                cpu_id = task_cpu_id;
            }

            cpu_id
        };

        let mut rq = self.rq[target_cpu as usize].lock_irq_disabled();
        if still_in_rq && let Err(_) = runnable.cpu().set_if_is_none(target_cpu) {
            return None;
        }
        rq.queue.push_back(runnable);

        Some(target_cpu)
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

struct FifoRunQueue<T: FifoSchedInfo> {
    current: Option<Arc<T>>,
    queue: VecDeque<Arc<T>>,
}

impl<T: FifoSchedInfo> FifoRunQueue<T> {
    pub const fn new() -> Self {
        Self {
            current: None,
            queue: VecDeque::new(),
        }
    }
}

impl<T: FifoSchedInfo> LocalRunQueue<T> for FifoRunQueue<T> {
    fn current(&self) -> Option<&Arc<T>> {
        self.current.as_ref()
    }

    fn update_current(&mut self, flags: super::UpdateFlags) -> bool {
        !matches!(flags, UpdateFlags::Tick)
    }

    fn pick_next_current(&mut self) -> Option<&Arc<T>> {
        let next_task = self.queue.pop_front()?;
        if let Some(prev_task) = self.current.replace(next_task) {
            self.queue.push_back(prev_task);
        }

        self.current.as_ref()
    }

    fn dequeue_current(&mut self) -> Option<Arc<T>> {
        self.current.take().inspect(|task| task.cpu().set_to_none())
    }
}

impl Default for FifoScheduler<Task> {
    fn default() -> Self {
        Self::new(num_cpus())
    }
}

impl FifoSchedInfo for Task {
    fn cpu(&self) -> &AtomicCpuId {
        self.cpu()
    }
}

trait FifoSchedInfo {
    fn cpu(&self) -> &AtomicCpuId;
}
