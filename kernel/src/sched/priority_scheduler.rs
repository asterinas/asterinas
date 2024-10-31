// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::Ordering;

use ostd::{
    cpu::{num_cpus, CpuId, CpuSet, PinCurrentCpu},
    task::{
        disable_preempt,
        scheduler::{
            info::CommonSchedInfo, inject_scheduler, EnqueueFlags, LocalRunQueue, Scheduler,
            UpdateFlags,
        },
        Task,
    },
    trap::disable_local,
};

use super::{
    priority::{Priority, PriorityRange},
    stats::{set_stats_from_scheduler, SchedulerStats},
};
use crate::{prelude::*, thread::Thread};

pub fn init() {
    let preempt_scheduler = Box::new(PreemptScheduler::default());
    let scheduler = Box::<PreemptScheduler<Thread, Task>>::leak(preempt_scheduler);

    // Inject the scheduler into the ostd for actual scheduling work.
    inject_scheduler(scheduler);

    // Set the scheduler into the system for statistics.
    // We set this after injecting the scheduler into ostd,
    // so that the loadavg statistics are updated after the scheduler is used.
    set_stats_from_scheduler(scheduler);
}

/// The preempt scheduler.
///
/// Real-time tasks are placed in the `real_time_entities` queue and
/// are always prioritized during scheduling.
/// Normal tasks are placed in the `normal_entities` queue and are only
/// scheduled for execution when there are no real-time tasks.
struct PreemptScheduler<T: PreemptSchedInfo + FromTask<U>, U: CommonSchedInfo> {
    rq: Vec<SpinLock<PreemptRunQueue<T, U>>>,
}

impl<T: PreemptSchedInfo + FromTask<U>, U: CommonSchedInfo> PreemptScheduler<T, U> {
    fn new(nr_cpus: usize) -> Self {
        let mut rq = Vec::with_capacity(nr_cpus);
        for _ in 0..nr_cpus {
            rq.push(SpinLock::new(PreemptRunQueue::new()));
        }
        Self { rq }
    }

    /// Selects a CPU for task to run on for the first time.
    fn select_cpu(&self, entity: &PreemptSchedEntity<T, U>) -> CpuId {
        // If the CPU of a runnable task has been set before, keep scheduling
        // the task to that one.
        // TODO: Consider migrating tasks between CPUs for load balancing.
        if let Some(cpu_id) = entity.task.cpu().get() {
            return cpu_id;
        }

        let irq_guard = disable_local();
        let mut selected = irq_guard.current_cpu();
        let mut minimum_load = usize::MAX;

        for candidate in entity.thread.cpu_affinity().iter() {
            let rq = self.rq[candidate.as_usize()].lock();
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

impl<T: Sync + Send + PreemptSchedInfo + FromTask<U>, U: Sync + Send + CommonSchedInfo> Scheduler<U>
    for PreemptScheduler<T, U>
{
    fn enqueue(&self, task: Arc<U>, flags: EnqueueFlags) -> Option<CpuId> {
        let entity = PreemptSchedEntity::new(task);

        let (still_in_rq, target_cpu) = {
            let selected_cpu_id = self.select_cpu(&entity);

            if let Err(task_cpu_id) = entity.task.cpu().set_if_is_none(selected_cpu_id) {
                debug_assert!(flags != EnqueueFlags::Spawn);
                (true, task_cpu_id)
            } else {
                (false, selected_cpu_id)
            }
        };

        let mut rq = self.rq[target_cpu.as_usize()].disable_irq().lock();
        if still_in_rq && let Err(_) = entity.task.cpu().set_if_is_none(target_cpu) {
            return None;
        }

        let new_priority = entity.thread.priority();

        if entity.thread.is_real_time() {
            rq.real_time_entities.push_back(entity);
        } else if entity.thread.is_lowest() {
            rq.lowest_entities.push_back(entity);
        } else {
            rq.normal_entities.push_back(entity);
        }

        // Preempt the current task, but only if the newly queued task has a strictly higher
        // priority (i.e., a lower value returned by the `priority` method) than the current task.
        if rq
            .current
            .as_ref()
            .is_some_and(|current| new_priority < current.thread.priority())
        {
            Some(target_cpu)
        } else {
            None
        }
    }

    fn local_rq_with(&self, f: &mut dyn FnMut(&dyn LocalRunQueue<U>)) {
        let irq_guard = disable_local();
        let local_rq: &PreemptRunQueue<T, U> = &self.rq[irq_guard.current_cpu().as_usize()].lock();
        f(local_rq);
    }

    fn local_mut_rq_with(&self, f: &mut dyn FnMut(&mut dyn LocalRunQueue<U>)) {
        let irq_guard = disable_local();
        let local_rq: &mut PreemptRunQueue<T, U> =
            &mut self.rq[irq_guard.current_cpu().as_usize()].lock();
        f(local_rq);
    }
}

impl<T: Sync + Send + PreemptSchedInfo + FromTask<U>, U: Sync + Send + CommonSchedInfo>
    SchedulerStats for PreemptScheduler<T, U>
{
    fn nr_queued_and_running(&self) -> (u32, u32) {
        let _preempt_guard = disable_preempt();
        let mut nr_queued = 0;
        let mut nr_running = 0;

        for rq in self.rq.iter() {
            let rq = rq.lock();

            nr_queued +=
                rq.real_time_entities.len() + rq.normal_entities.len() + rq.lowest_entities.len();

            if rq.current.is_some() {
                nr_running += 1;
            }
        }

        (nr_queued as u32, nr_running)
    }
}

impl Default for PreemptScheduler<Thread, Task> {
    fn default() -> Self {
        Self::new(num_cpus())
    }
}

struct PreemptRunQueue<T: PreemptSchedInfo + FromTask<U>, U: CommonSchedInfo> {
    current: Option<PreemptSchedEntity<T, U>>,
    real_time_entities: VecDeque<PreemptSchedEntity<T, U>>,
    normal_entities: VecDeque<PreemptSchedEntity<T, U>>,
    lowest_entities: VecDeque<PreemptSchedEntity<T, U>>,
}

impl<T: PreemptSchedInfo + FromTask<U>, U: CommonSchedInfo> PreemptRunQueue<T, U> {
    pub fn new() -> Self {
        Self {
            current: None,
            real_time_entities: VecDeque::new(),
            normal_entities: VecDeque::new(),
            lowest_entities: VecDeque::new(),
        }
    }
}

impl<T: PreemptSchedInfo + FromTask<U>, U: CommonSchedInfo> LocalRunQueue<U>
    for PreemptRunQueue<T, U>
{
    fn current(&self) -> Option<&Arc<U>> {
        self.current.as_ref().map(|entity| &entity.task)
    }

    fn update_current(&mut self, flags: UpdateFlags) -> bool {
        match flags {
            UpdateFlags::Tick => {
                let Some(ref mut current_entity) = self.current else {
                    return false;
                };
                current_entity.tick()
                    || (!current_entity.thread.is_real_time()
                        && !self.real_time_entities.is_empty())
            }
            _ => true,
        }
    }

    fn pick_next_current(&mut self) -> Option<&Arc<U>> {
        let next_entity = if !self.real_time_entities.is_empty() {
            self.real_time_entities.pop_front()
        } else if !self.normal_entities.is_empty() {
            self.normal_entities.pop_front()
        } else {
            self.lowest_entities.pop_front()
        }?;
        if let Some(prev_entity) = self.current.replace(next_entity) {
            if prev_entity.thread.is_real_time() {
                self.real_time_entities.push_back(prev_entity);
            } else if prev_entity.thread.is_lowest() {
                self.lowest_entities.push_back(prev_entity);
            } else {
                self.normal_entities.push_back(prev_entity);
            }
        }

        Some(&self.current.as_ref().unwrap().task)
    }

    fn dequeue_current(&mut self) -> Option<Arc<U>> {
        self.current.take().map(|entity| {
            let runnable = entity.task;
            runnable.cpu().set_to_none();

            runnable
        })
    }
}
struct PreemptSchedEntity<T: PreemptSchedInfo + FromTask<U>, U: CommonSchedInfo> {
    task: Arc<U>,
    thread: Arc<T>,
    time_slice: TimeSlice,
}

impl<T: PreemptSchedInfo + FromTask<U>, U: CommonSchedInfo> PreemptSchedEntity<T, U> {
    fn new(task: Arc<U>) -> Self {
        let thread = T::from_task(&task);
        let time_slice = TimeSlice::default();
        Self {
            task,
            thread,
            time_slice,
        }
    }

    fn tick(&mut self) -> bool {
        self.time_slice.elapse()
    }
}

impl<T: PreemptSchedInfo + FromTask<U>, U: CommonSchedInfo> Clone for PreemptSchedEntity<T, U> {
    fn clone(&self) -> Self {
        Self {
            task: self.task.clone(),
            thread: self.thread.clone(),
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

impl PreemptSchedInfo for Thread {
    const REAL_TIME_TASK_PRIORITY: Priority = Priority::new(PriorityRange::new(100));
    const LOWEST_TASK_PRIORITY: Priority = Priority::new(PriorityRange::new(PriorityRange::MAX));

    fn priority(&self) -> Priority {
        self.atomic_priority().load(Ordering::Relaxed)
    }

    fn cpu_affinity(&self) -> CpuSet {
        self.atomic_cpu_affinity().load()
    }
}

trait PreemptSchedInfo {
    const REAL_TIME_TASK_PRIORITY: Priority;
    const LOWEST_TASK_PRIORITY: Priority;

    fn priority(&self) -> Priority;

    fn cpu_affinity(&self) -> CpuSet;

    fn is_real_time(&self) -> bool {
        self.priority() < Self::REAL_TIME_TASK_PRIORITY
    }

    fn is_lowest(&self) -> bool {
        self.priority() == Self::LOWEST_TASK_PRIORITY
    }
}

impl FromTask<Task> for Thread {
    fn from_task(task: &Arc<Task>) -> Arc<Self> {
        task.data().downcast_ref::<Arc<Self>>().unwrap().clone()
    }
}

trait FromTask<U> {
    fn from_task(task: &Arc<U>) -> Arc<Self>;
}
