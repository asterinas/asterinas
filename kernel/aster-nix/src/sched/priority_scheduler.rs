// SPDX-License-Identifier: MPL-2.0

use ostd::{
    cpu::{num_cpus, this_cpu},
    task::{inject_scheduler, EnqueueFlags, LocalRunQueue, Scheduler, Task, UpdateFlags},
};

use crate::prelude::*;

pub fn init() {
    let preempt_scheduler = Box::new(PreemptScheduler::new());
    let scheduler = Box::<PreemptScheduler<Task>>::leak(preempt_scheduler);
    inject_scheduler(scheduler);
}

/// The preempt scheduler
///
/// Real-time tasks are placed in the `real_time_tasks` queue and
/// are always prioritized during scheduling.
/// Normal tasks are placed in the `normal_tasks` queue and are only
/// scheduled for execution when there are no real-time tasks.
struct PreemptScheduler<T: PreemptSchedEntity> {
    idx: SpinLock<usize>,
    rq: Vec<SpinLock<PreemptRunQueue<T>>>,
}

trait PreemptSchedEntity {
    const REAL_TIME_TASK_PRIORITY: u16 = 100;

    fn is_real_time(&self) -> bool;

    fn can_run_on(&self, cpu_id: u32) -> bool;
}

impl PreemptSchedEntity for Task {
    fn is_real_time(&self) -> bool {
        self.priority().get() < Self::REAL_TIME_TASK_PRIORITY
    }

    fn can_run_on(&self, cpu_id: u32) -> bool {
        self.cpu_affinity().contains(cpu_id)
    }
}

impl<T: PreemptSchedEntity> PreemptScheduler<T> {
    pub fn new() -> Self {
        let mut rq = Vec::new();
        for _ in 0..num_cpus() {
            rq.push(SpinLock::new(PreemptRunQueue::new()));
        }
        Self {
            idx: SpinLock::new(0),
            rq,
        }
    }
}

impl<T: Sync + Send + PreemptSchedEntity> Scheduler<T> for PreemptScheduler<T> {
    fn enqueue(&self, runnable: Arc<T>, _flags: EnqueueFlags) {
        let mut idx = self.idx.lock_irq_disabled();
        for _ in 0..self.rq.len() {
            if runnable.can_run_on((*idx).try_into().unwrap()) {
                if runnable.is_real_time() {
                    self.rq[*idx]
                        .lock_irq_disabled()
                        .real_time_tasks
                        .push_back(runnable);
                } else {
                    self.rq[*idx]
                        .lock_irq_disabled()
                        .normal_tasks
                        .push_back(runnable);
                }
                break;
            }

            *idx = if *idx == self.rq.len() - 1 {
                0
            } else {
                *idx + 1
            };
        }
        *idx = if *idx == self.rq.len() - 1 {
            0
        } else {
            *idx + 1
        };
    }

    fn local_rq_with(&self, f: &mut dyn FnMut(&dyn LocalRunQueue<T>)) {
        let local_rq: &PreemptRunQueue<T> = &self.rq[this_cpu() as usize].lock_irq_disabled();
        f(local_rq);
    }

    fn local_mut_rq_with(&self, f: &mut dyn FnMut(&mut dyn LocalRunQueue<T>)) {
        let local_rq: &mut PreemptRunQueue<T> =
            &mut self.rq[this_cpu() as usize].lock_irq_disabled();
        f(local_rq);
    }
}

struct PreemptRunQueue<T: PreemptSchedEntity> {
    current: Option<Arc<T>>,
    /// Tasks with a priority of less than 100 are regarded as real-time tasks.
    real_time_tasks: VecDeque<Arc<T>>,
    /// Tasks with a priority greater than or equal to 100 are regarded as normal tasks.
    normal_tasks: VecDeque<Arc<T>>,
    should_preempt: bool,
}

impl<T: PreemptSchedEntity> PreemptRunQueue<T> {
    pub fn new() -> Self {
        Self {
            current: None,
            real_time_tasks: VecDeque::new(),
            normal_tasks: VecDeque::new(),
            should_preempt: false,
        }
    }
}

impl<T: Sync + Send + PreemptSchedEntity> LocalRunQueue<T> for PreemptRunQueue<T> {
    fn update_current(&mut self, flags: UpdateFlags) -> bool {
        match flags {
            UpdateFlags::Tick => {
                if let Some(current) = &self.current {
                    !current.is_real_time() && !self.real_time_tasks.is_empty()
                } else {
                    true
                }
            }
            _ => true,
        }
    }

    fn dequeue_current(&mut self) -> Option<Arc<T>> {
        self.current.take()
    }

    fn pick_next_current(&mut self) -> Option<Arc<T>> {
        if !self.real_time_tasks.is_empty() {
            self.real_time_tasks.pop_front()
        } else {
            self.normal_tasks.pop_front()
        }
    }

    fn set_current(&mut self, next: Option<Arc<T>>) {
        let prev = self.current.take();
        self.current = next;
        if let Some(prev_task) = prev {
            if prev_task.is_real_time() {
                self.real_time_tasks.push_back(prev_task)
            } else {
                self.normal_tasks.push_back(prev_task);
            }
        }
    }

    fn current(&self) -> Option<&Arc<T>> {
        self.current.as_ref()
    }

    fn set_should_preempt(&mut self, should_preempt: bool) {
        self.should_preempt = should_preempt
    }

    fn should_preempt(&self) -> bool {
        self.should_preempt
    }
}
