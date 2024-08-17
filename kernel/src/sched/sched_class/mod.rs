// SPDX-License-Identifier: MPL-2.0

#![warn(unused)]

use alloc::{
    boxed::Box,
    sync::{Arc, Weak},
};
use core::fmt;

use ostd::{
    cpu::{num_cpus, CpuSet, PinCurrentCpu},
    sync::SpinLock,
    task::{
        scheduler::{inject_scheduler, EnqueueFlags, LocalRunQueue, Scheduler, UpdateFlags},
        Task,
    },
    trap::disable_local,
};

mod time;

mod fair;
mod idle;
mod real_time;
mod stop;

use ostd::arch::read_tsc as sched_clock;

use super::priority::Priority;
use crate::thread::Thread;

#[allow(unused)]
pub fn init() {
    inject_scheduler(Box::leak(Box::new(ClassScheduler::default())));
}

/// Represents the middle layer between scheduling classes and generic scheduler
/// traits. It consists of all the sets of run queues for CPU cores. Other global
/// information may also be stored here.
pub struct ClassScheduler {
    rqs: Box<[SpinLock<PerCpuClassRqSet>]>,
}

/// Represents the run queue for each CPU core. It stores a list of run queues for
/// scheduling classes in its corresponding CPU core. The current task of this CPU
/// core is also stored in this structure.
struct PerCpuClassRqSet {
    stop: stop::StopClassRq,
    real_time: real_time::RealTimeClassRq,
    fair: fair::FairClassRq,
    idle: idle::IdleClassRq,
    current: Option<Arc<Task>>,
}

/// The run queue for scheduling classes (the main trait). Scheduling classes
/// should implement this trait to function as expected.
trait SchedClassRq: Send + fmt::Debug {
    type Entity;

    /// Enqueues a task into the run queue.
    fn enqueue(&mut self, thread: Arc<Thread>, entity: &Self::Entity);

    /// Dequeues a task from the run queue.
    fn dequeue(&mut self, entity: &Self::Entity);

    /// Picks the next task for running.
    fn pick_next(&mut self) -> Option<Arc<Thread>>;

    /// Update the information of the current task.
    fn update_current(&mut self, entity: &mut Self::Entity, flags: UpdateFlags) -> bool;
}

/// The scheduling entity. Users should not construct a scheduling entity
/// directly using its variant types.
pub enum SchedEntity {
    Stop(stop::StopEntity),
    RealTime(real_time::RealTimeEntity),
    Fair(fair::VRuntime),
    Idle(idle::IdleEntity),
}

impl SchedEntity {
    /// Constructs a new scheduling entity object based from the given priority.
    pub fn new(priority: Priority) -> SchedEntity {
        match priority.range().get() {
            0 => SchedEntity::Stop(stop::StopEntity(())),
            1..100 => SchedEntity::RealTime(real_time::RealTimeEntity::new(priority)),
            100..=139 => SchedEntity::Fair(fair::VRuntime::new(priority.into())),
            _ => SchedEntity::Idle(idle::IdleEntity(())),
        }
    }
}

impl Scheduler for ClassScheduler {
    fn enqueue(&self, task: Arc<Task>, _flags: EnqueueFlags) -> Option<u32> {
        let thread = Thread::borrow_from_task(&task);

        let cpu_affinity = thread.lock_cpu_affinity();
        let cpu = self.select_cpu(&cpu_affinity);
        task.schedule_info().cpu.set_if_is_none(cpu).ok()?;
        drop(cpu_affinity);

        let mut rq = self.rqs[cpu as usize].disable_irq().lock();
        rq.enqueue_thread(thread);
        Some(cpu)
    }

    fn local_mut_rq_with(&self, f: &mut dyn FnMut(&mut dyn LocalRunQueue)) {
        let guard = disable_local();
        let mut lock = self.rqs[guard.current_cpu() as usize].lock();
        f(&mut *lock)
    }

    fn local_rq_with(&self, f: &mut dyn FnMut(&dyn LocalRunQueue)) {
        let guard = disable_local();
        f(&*self.rqs[guard.current_cpu() as usize].lock())
    }
}

impl ClassScheduler {
    fn select_cpu(&self, affinity: &CpuSet) -> u32 {
        let guard = disable_local();
        let cur = guard.current_cpu();
        if affinity.contains(cur) {
            cur
        } else {
            affinity.iter().next().expect("empty affinity")
        }
    }
}

impl PerCpuClassRqSet {
    fn pick_next_thread(&mut self) -> Option<Arc<Thread>> {
        (self.stop.pick_next())
            .or_else(|| self.real_time.pick_next())
            .or_else(|| self.fair.pick_next())
            .or_else(|| self.idle.pick_next())
    }

    fn enqueue_thread(&mut self, thread: &Arc<Thread>) {
        let cloned = thread.clone();
        match &*thread.sched_entity().lock() {
            SchedEntity::Stop(stop_entity) => self.stop.enqueue(cloned, stop_entity),
            SchedEntity::RealTime(real_time_entity) => {
                self.real_time.enqueue(cloned, real_time_entity)
            }
            SchedEntity::Fair(vruntime) => self.fair.enqueue(cloned, vruntime),
            SchedEntity::Idle(idle_entity) => self.idle.enqueue(cloned, idle_entity),
        }
    }
}

impl LocalRunQueue for PerCpuClassRqSet {
    fn current(&self) -> Option<&Arc<Task>> {
        self.current.as_ref()
    }

    fn pick_next_current(&mut self) -> Option<&Arc<Task>> {
        match self.pick_next_thread() {
            Some(next) => {
                let next_task = next.task();
                if let Some(old_task) = self.current.replace(next_task.upgrade().unwrap()) {
                    if Arc::as_ptr(&old_task) == Weak::as_ptr(next_task) {
                        return None;
                    }
                    let old = Thread::borrow_from_task(&old_task);
                    self.enqueue_thread(old);
                }
                self.current.as_ref()
            }
            None => None,
        }
    }

    fn update_current(&mut self, flags: UpdateFlags) -> bool {
        if let Some(cur_task) = &self.current {
            let cur = Thread::borrow_from_task(cur_task);
            match &mut *cur.sched_entity().lock() {
                SchedEntity::Stop(stop_entity) => self.stop.update_current(stop_entity, flags),
                SchedEntity::RealTime(real_time_entity) => {
                    self.real_time.update_current(real_time_entity, flags)
                }
                SchedEntity::Fair(vruntime) => self.fair.update_current(vruntime, flags),
                SchedEntity::Idle(idle_entity) => self.idle.update_current(idle_entity, flags),
            }
        } else {
            true
        }
    }

    fn dequeue_current(&mut self) -> Option<Arc<Task>> {
        self.current.take().map(|cur_task| {
            let cur = Thread::borrow_from_task(&cur_task);
            match &*cur.sched_entity().lock() {
                SchedEntity::Stop(stop_entity) => self.stop.dequeue(stop_entity),
                SchedEntity::RealTime(real_time_entity) => self.real_time.dequeue(real_time_entity),
                SchedEntity::Fair(vruntime) => self.fair.dequeue(vruntime),
                SchedEntity::Idle(idle_entity) => self.idle.dequeue(idle_entity),
            }
            cur_task.schedule_info().cpu.set_to_none();
            cur_task
        })
    }
}

impl Default for ClassScheduler {
    fn default() -> Self {
        let class_rq = |cpu| {
            SpinLock::new(PerCpuClassRqSet {
                stop: stop::new_class(cpu),
                real_time: real_time::new_class(cpu),
                fair: fair::new_class(cpu),
                idle: idle::new_class(cpu),
                current: None,
            })
        };
        ClassScheduler {
            rqs: (0..num_cpus()).map(class_rq).collect(),
        }
    }
}
