// SPDX-License-Identifier: MPL-2.0

use ostd::{
    cpu::{num_cpus, CpuSet, PinCurrentCpu},
    task::{
        scheduler::{inject_scheduler, EnqueueFlags, LocalRunQueue, Scheduler, UpdateFlags},
        AtomicCpuId, Priority, Task,
    },
    trap::disable_local,
};

use crate::prelude::*;

pub fn init() {
    let preempt_scheduler = Box::new(PreemptScheduler::default());
    let scheduler = Box::<PreemptScheduler<Task>>::leak(preempt_scheduler);
    inject_scheduler(scheduler);
}

/// The preempt scheduler.
///
/// Real-time tasks are placed in the `real_time_entities` queue and
/// are always prioritized during scheduling.
/// Normal tasks are placed in the `normal_entities` queue and are only
/// scheduled for execution when there are no real-time tasks.
struct PreemptScheduler<T: PreemptSchedInfo> {
    rq: Vec<SpinLock<PreemptRunQueue<T>>>,
}

impl<T: PreemptSchedInfo> PreemptScheduler<T> {
    fn new(nr_cpus: u32) -> Self {
        let mut rq = Vec::with_capacity(nr_cpus as usize);
        for _ in 0..nr_cpus {
            rq.push(SpinLock::new(PreemptRunQueue::new()));
        }
        Self { rq }
    }

    /// Selects a CPU for task to run on for the first time.
    fn select_cpu(&self, runnable: &Arc<T>) -> u32 {
        // If the CPU of a runnable task has been set before, keep scheduling
        // the task to that one.
        // TODO: Consider migrating tasks between CPUs for load balancing.
        if let Some(cpu_id) = runnable.cpu().get() {
            return cpu_id;
        }

        let irq_guard = disable_local();
        let mut selected = irq_guard.current_cpu();
        let mut minimum_load = usize::MAX;

        for candidate in runnable.cpu_affinity().iter() {
            let rq = self.rq[candidate as usize].lock();
            // A wild guess measuring the load of a runqueue. We assume that
            // real-time tasks are 4-times as important as normal tasks.
            let load = rq.real_time_entities.len() * 8
                + rq.normal_entities.len() * 2
                + rq.lowest_entities.len();
            if load < minimum_load {
                selected = candidate;
                minimum_load = load;
            }
        }

        selected
    }
}

impl<T: Sync + Send + PreemptSchedInfo> Scheduler<T> for PreemptScheduler<T> {
    fn enqueue(&self, runnable: Arc<T>, flags: EnqueueFlags) -> Option<u32> {
        let mut still_in_rq = false;
        let target_cpu = {
            let mut cpu_id = self.select_cpu(&runnable);
            if let Err(task_cpu_id) = runnable.cpu().set_if_is_none(cpu_id) {
                debug_assert!(flags != EnqueueFlags::Spawn);
                still_in_rq = true;
                cpu_id = task_cpu_id;
            }

            cpu_id
        };

        let mut rq = self.rq[target_cpu as usize].disable_irq().lock();
        if still_in_rq && let Err(_) = runnable.cpu().set_if_is_none(target_cpu) {
            return None;
        }
        let entity = PreemptSchedEntity::new(runnable);
        if entity.is_real_time() {
            rq.real_time_entities.push_back(entity);
        } else if entity.is_lowest() {
            rq.lowest_entities.push_back(entity);
        } else {
            rq.normal_entities.push_back(entity);
        }

        Some(target_cpu)
    }

    fn local_rq_with(&self, f: &mut dyn FnMut(&dyn LocalRunQueue<T>)) {
        let irq_guard = disable_local();
        let local_rq: &PreemptRunQueue<T> = &self.rq[irq_guard.current_cpu() as usize].lock();
        f(local_rq);
    }

    fn local_mut_rq_with(&self, f: &mut dyn FnMut(&mut dyn LocalRunQueue<T>)) {
        let irq_guard = disable_local();
        let local_rq: &mut PreemptRunQueue<T> =
            &mut self.rq[irq_guard.current_cpu() as usize].lock();
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
    real_time_entities: VecDeque<PreemptSchedEntity<T>>,
    normal_entities: VecDeque<PreemptSchedEntity<T>>,
    lowest_entities: VecDeque<PreemptSchedEntity<T>>,
}

impl<T: PreemptSchedInfo> PreemptRunQueue<T> {
    pub fn new() -> Self {
        Self {
            current: None,
            real_time_entities: VecDeque::new(),
            normal_entities: VecDeque::new(),
            lowest_entities: VecDeque::new(),
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
                let Some(ref mut current_entity) = self.current else {
                    return false;
                };
                current_entity.tick()
                    || (!current_entity.is_real_time() && !self.real_time_entities.is_empty())
            }
            _ => true,
        }
    }

    fn pick_next_current(&mut self) -> Option<&Arc<T>> {
        let next_entity = if !self.real_time_entities.is_empty() {
            self.real_time_entities.pop_front()
        } else if !self.normal_entities.is_empty() {
            self.normal_entities.pop_front()
        } else {
            self.lowest_entities.pop_front()
        }?;
        if let Some(prev_entity) = self.current.replace(next_entity) {
            if prev_entity.is_real_time() {
                self.real_time_entities.push_back(prev_entity);
            } else if prev_entity.is_lowest() {
                self.lowest_entities.push_back(prev_entity);
            } else {
                self.normal_entities.push_back(prev_entity);
            }
        }

        Some(&self.current.as_ref().unwrap().runnable)
    }

    fn dequeue_current(&mut self) -> Option<Arc<T>> {
        self.current.take().map(|entity| {
            let runnable = entity.runnable;
            runnable.cpu().set_to_none();

            runnable
        })
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

    fn is_lowest(&self) -> bool {
        self.runnable.is_lowest()
    }

    fn tick(&mut self) -> bool {
        self.time_slice.elapse()
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
    elapsed_ticks: u32,
}

impl TimeSlice {
    const DEFAULT_TIME_SLICE: u32 = 100;

    pub const fn new() -> Self {
        TimeSlice { elapsed_ticks: 0 }
    }

    pub fn elapse(&mut self) -> bool {
        self.elapsed_ticks = (self.elapsed_ticks + 1) % Self::DEFAULT_TIME_SLICE;

        self.elapsed_ticks == 0
    }
}

impl Default for TimeSlice {
    fn default() -> Self {
        Self::new()
    }
}

impl PreemptSchedInfo for Task {
    type PRIORITY = Priority;

    const REAL_TIME_TASK_PRIORITY: Self::PRIORITY = Priority::new(100);
    const LOWEST_TASK_PRIORITY: Self::PRIORITY = Priority::lowest();

    fn priority(&self) -> Self::PRIORITY {
        self.schedule_info().priority
    }

    fn cpu(&self) -> &AtomicCpuId {
        &self.schedule_info().cpu
    }

    fn cpu_affinity(&self) -> &CpuSet {
        &self.schedule_info().cpu_affinity
    }
}

trait PreemptSchedInfo {
    type PRIORITY: Ord + PartialOrd + Eq + PartialEq;

    const REAL_TIME_TASK_PRIORITY: Self::PRIORITY;
    const LOWEST_TASK_PRIORITY: Self::PRIORITY;

    fn priority(&self) -> Self::PRIORITY;

    fn cpu(&self) -> &AtomicCpuId;

    fn cpu_affinity(&self) -> &CpuSet;

    fn is_real_time(&self) -> bool {
        self.priority() < Self::REAL_TIME_TASK_PRIORITY
    }

    fn is_lowest(&self) -> bool {
        self.priority() == Self::LOWEST_TASK_PRIORITY
    }
}
