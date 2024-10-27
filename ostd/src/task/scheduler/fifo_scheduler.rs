// SPDX-License-Identifier: MPL-2.0

use alloc::{boxed::Box, collections::VecDeque, sync::Arc, vec::Vec};

use super::{
    info::CommonSchedInfo, inject_scheduler, EnqueueFlags, LocalRunQueue, Scheduler, UpdateFlags,
};
use crate::{
    cpu::{num_cpus, CpuId, PinCurrentCpu},
    sync::SpinLock,
    task::{disable_preempt, Task},
};

pub fn init() {
    let fifo_scheduler = Box::new(FifoScheduler::default());
    let scheduler = Box::<FifoScheduler<Task>>::leak(fifo_scheduler);
    inject_scheduler(scheduler);
}

/// A simple FIFO (First-In-First-Out) task scheduler.
struct FifoScheduler<T: CommonSchedInfo> {
    /// A thread-safe queue to hold tasks waiting to be executed.
    rq: Vec<SpinLock<FifoRunQueue<T>>>,
}

impl<T: CommonSchedInfo> FifoScheduler<T> {
    /// Creates a new instance of `FifoScheduler`.
    fn new(nr_cpus: usize) -> Self {
        let mut rq = Vec::new();
        for _ in 0..nr_cpus {
            rq.push(SpinLock::new(FifoRunQueue::new()));
        }
        Self { rq }
    }

    fn select_cpu(&self) -> CpuId {
        // FIXME: adopt more reasonable policy once we fully enable SMP.
        CpuId::bsp()
    }
}

impl<T: CommonSchedInfo + Send + Sync> Scheduler<T> for FifoScheduler<T> {
    fn enqueue(&self, runnable: Arc<T>, flags: EnqueueFlags) -> Option<CpuId> {
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

        self.rq[target_cpu.as_usize()]
            .disable_irq()
            .lock_with(|rq| {
                if still_in_rq && let Err(_) = runnable.cpu().set_if_is_none(target_cpu) {
                    return None;
                }
                rq.queue.push_back(runnable);

                Some(target_cpu)
            })
    }

    fn local_rq_with(&self, f: &mut dyn FnMut(&dyn LocalRunQueue<T>)) {
        let preempt_guard = disable_preempt();
        self.rq[preempt_guard.current_cpu().as_usize()]
            .disable_irq()
            .lock_with(|rq| f(rq));
    }

    fn local_mut_rq_with(&self, f: &mut dyn FnMut(&mut dyn LocalRunQueue<T>)) {
        let preempt_guard = disable_preempt();
        self.rq[preempt_guard.current_cpu().as_usize()]
            .disable_irq()
            .lock_with(|rq| f(rq));
    }
}

struct FifoRunQueue<T: CommonSchedInfo> {
    current: Option<Arc<T>>,
    queue: VecDeque<Arc<T>>,
}

impl<T: CommonSchedInfo> FifoRunQueue<T> {
    pub const fn new() -> Self {
        Self {
            current: None,
            queue: VecDeque::new(),
        }
    }
}

impl<T: CommonSchedInfo> LocalRunQueue<T> for FifoRunQueue<T> {
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
