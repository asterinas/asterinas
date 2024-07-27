// SPDX-License-Identifier: MPL-2.0

use ostd::{
    cpu::{num_cpus, this_cpu},
    task::{
        scheduler::{inject_scheduler, EnqueueFlags, LocalRunQueue, Scheduler, UpdateFlags},
        Priority, Task,
    },
};

use crate::prelude::*;

pub fn init() {
    let preempt_scheduler = Box::new(PreemptScheduler::default());
    let scheduler = Box::<PreemptScheduler<Task>>::leak(preempt_scheduler);
    inject_scheduler(scheduler);
}

/// The preempt scheduler
///
/// Real-time tasks are placed in the `real_time_tasks` queue and
/// are always prioritized during scheduling.
/// Normal tasks are placed in the `normal_tasks` queue and are only
/// scheduled for execution when there are no real-time tasks.
struct PreemptScheduler<T: PreemptSchedInfo> {
    rq: Vec<SpinLock<PreemptRunQueue<T>>>,
}

impl<T: PreemptSchedInfo> PreemptScheduler<T> {
    fn new(ncpu: u32) -> Self {
        let mut rq = Vec::with_capacity(ncpu as usize);
        for _ in 0..ncpu {
            rq.push(SpinLock::new(PreemptRunQueue::new()));
        }
        Self { rq }
    }

    /// Selects a cpu for task to run on.
    fn select_cpu(&self, _runnable: &Arc<T>) -> usize {
        // FIXME: adopt more reasonable policy once we fully enable SMP.
        0
    }
}

impl<T: Sync + Send + PreemptSchedInfo> Scheduler<T> for PreemptScheduler<T> {
    fn enqueue(&self, runnable: Arc<T>, flags: EnqueueFlags) -> Option<u32> {
        if flags == EnqueueFlags::Wake {
            for cpu_id in 0..self.rq.len() {
                let rq = self.rq[cpu_id].lock_irq_disabled();
                if rq.current.is_none() {
                    continue;
                }
                if Arc::ptr_eq(&runnable, &rq.current.as_ref().unwrap().runnable) {
                    return Some(cpu_id as u32);
                }
            }
        }
        let target_cpu = self.select_cpu(&runnable);
        let entity = PreemptSchedEntity::new(runnable);
        let mut rq = self.rq[target_cpu].lock_irq_disabled();
        if entity.is_real_time() {
            rq.real_time_tasks.push_back(entity);
        } else {
            rq.normal_tasks.push_back(entity);
        }

        Some(target_cpu as u32)
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

impl Default for PreemptScheduler<Task> {
    fn default() -> Self {
        Self::new(num_cpus())
    }
}

struct PreemptRunQueue<T: PreemptSchedInfo> {
    current: Option<PreemptSchedEntity<T>>,
    /// Tasks with a priority of less than 100 are regarded as real-time tasks.
    real_time_tasks: VecDeque<PreemptSchedEntity<T>>,
    /// Tasks with a priority greater than or equal to 100 are regarded as normal tasks.
    normal_tasks: VecDeque<PreemptSchedEntity<T>>,
}

impl<T: PreemptSchedInfo> PreemptRunQueue<T> {
    pub fn new() -> Self {
        Self {
            current: None,
            real_time_tasks: VecDeque::new(),
            normal_tasks: VecDeque::new(),
        }
    }
}

impl<T: Sync + Send + PreemptSchedInfo> LocalRunQueue<T> for PreemptRunQueue<T> {
    fn current(&self) -> Option<&Arc<T>> {
        self.current.as_ref().map(|entity| &entity.runnable)
    }

    fn update_current(&mut self, flags: UpdateFlags) -> bool {
        match flags {
            UpdateFlags::Tick => {
                self.current.as_mut().unwrap().ran_out_of_time()
                    || (!self.current.as_ref().unwrap().is_real_time()
                        && !self.real_time_tasks.is_empty())
            }
            _ => true,
        }
    }

    fn pick_next_current(&mut self, keep_current: bool) -> Option<Arc<T>> {
        let prev = self.current.take();
        let next = if !self.real_time_tasks.is_empty() {
            self.real_time_tasks.pop_front()
        } else {
            self.normal_tasks.pop_front()
        };
        self.current = next.clone();
        if self.current.is_none() {
            self.current = prev;
        } else if keep_current && let Some(prev_entity) = prev {
            if prev_entity.is_real_time() {
                self.real_time_tasks.push_back(prev_entity);
            } else {
                self.normal_tasks.push_back(prev_entity);
            }
        }

        next.map(|entity| entity.runnable)
    }
}

struct PreemptSchedEntity<T: PreemptSchedInfo> {
    runnable: Arc<T>,
    time_slice: TimeSlice,
}

impl<T: PreemptSchedInfo> PreemptSchedEntity<T> {
    fn new(runnable: Arc<T>) -> Self {
        Self {
            runnable,
            time_slice: TimeSlice::default(),
        }
    }

    fn is_real_time(&self) -> bool {
        self.runnable.is_real_time()
    }

    fn ran_out_of_time(&mut self) -> bool {
        self.time_slice.ran_out()
    }
}

impl<T: PreemptSchedInfo> Clone for PreemptSchedEntity<T> {
    fn clone(&self) -> Self {
        Self {
            runnable: self.runnable.clone(),
            time_slice: self.time_slice,
        }
    }
}

#[derive(Clone, Copy)]
pub struct TimeSlice {
    count: u8,
}

impl TimeSlice {
    const DEFAULT_TIME_SLICE: u8 = 100;

    pub const fn new() -> Self {
        TimeSlice { count: 0 }
    }

    pub fn ran_out(&mut self) -> bool {
        self.count = (self.count + 1) % Self::DEFAULT_TIME_SLICE;

        self.count == 0
    }
}

impl Default for TimeSlice {
    fn default() -> Self {
        Self::new()
    }
}

trait PreemptSchedInfo {
    type PRIORITY: Ord + PartialOrd + Eq + PartialEq;

    const REAL_TIME_TASK_PRIORITY: Self::PRIORITY;

    fn priority(&self) -> Self::PRIORITY;

    fn is_real_time(&self) -> bool {
        self.priority() < Self::REAL_TIME_TASK_PRIORITY
    }
}

impl PreemptSchedInfo for Task {
    type PRIORITY = Priority;

    const REAL_TIME_TASK_PRIORITY: Self::PRIORITY = Priority::new(100);

    fn priority(&self) -> Self::PRIORITY {
        self.priority()
    }
}
