// SPDX-License-Identifier: MPL-2.0

#![warn(unused)]

use alloc::{boxed::Box, sync::Arc};
use core::{fmt, sync::atomic::AtomicU64};

use ostd::{
    cpu::{all_cpus, AtomicCpuSet, CpuId, PinCurrentCpu},
    sync::SpinLock,
    task::{
        scheduler::{
            info::CommonSchedInfo, inject_scheduler, EnqueueFlags, LocalRunQueue, Scheduler,
            UpdateFlags,
        },
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

use super::{
    priority::{Nice, Priority, RangedU8},
    stats::SchedulerStats,
};
use crate::thread::{AsThread, Thread};

#[allow(unused)]
pub fn init() {
    inject_scheduler(Box::leak(Box::new(ClassScheduler::new())));
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
    stop: Arc<stop::StopClassRq>,
    real_time: real_time::RealTimeClassRq,
    fair: fair::FairClassRq,
    idle: idle::IdleClassRq,
    current: Option<(Arc<Task>, CurrentRuntime)>,
}

/// Stores the runtime information of the current task.
///
/// This is used to calculate the time slice of the current task.
///
/// This struct is independent of the current `Arc<Task>` instead encapsulating the
/// task, because the scheduling class implementations use `CurrentRuntime` and
/// `SchedAttr` only.
struct CurrentRuntime {
    start: u64,
    delta: u64,
    period_delta: u64,
}

impl CurrentRuntime {
    fn new() -> Self {
        CurrentRuntime {
            start: sched_clock(),
            delta: 0,
            period_delta: 0,
        }
    }

    fn update(&mut self) {
        let now = sched_clock();
        self.delta = now - core::mem::replace(&mut self.start, now);
        self.period_delta += self.delta;
    }
}

/// The run queue for scheduling classes (the main trait). Scheduling classes
/// should implement this trait to function as expected.
trait SchedClassRq: Send + fmt::Debug {
    /// Enqueues a task into the run queue.
    fn enqueue(&mut self, thread: Arc<Thread>, flags: Option<EnqueueFlags>);

    /// Returns the number of threads in the run queue.
    fn len(&mut self) -> usize;

    /// Checks if the run queue is empty.
    fn is_empty(&mut self) -> bool {
        self.len() == 0
    }

    /// Picks the next task for running.
    fn pick_next(&mut self) -> Option<Arc<Thread>>;

    /// Update the information of the current task.
    fn update_current(&mut self, rt: &CurrentRuntime, attr: &SchedAttr, flags: UpdateFlags)
        -> bool;
}

pub use real_time::RealTimePolicy;

/// The User-chosen scheduling policy.
///
/// The scheduling policies are specified by the user, usually through its priority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SchedPolicy {
    Stop,
    RealTime {
        rt_prio: real_time::RtPrio,
        rt_policy: RealTimePolicy,
    },
    Fair(Nice),
    Idle,
}

impl From<Priority> for SchedPolicy {
    fn from(priority: Priority) -> Self {
        match priority.range().get() {
            0 => SchedPolicy::Stop,
            rt @ 1..=99 => SchedPolicy::RealTime {
                rt_prio: RangedU8::new(rt),
                rt_policy: Default::default(),
            },
            100..=139 => SchedPolicy::Fair(priority.into()),
            _ => SchedPolicy::Idle,
        }
    }
}

/// The scheduling attribute for a thread.
///
/// This is used to store the scheduling policy and runtime parameters for each
/// scheduling class.
#[derive(Debug)]
pub struct SchedAttr {
    policy: SpinLock<SchedPolicy>,

    real_time: real_time::RealTimeAttr,
    fair: fair::FairAttr,
}

impl SchedAttr {
    /// Constructs a new `SchedAttr` with the given scheduling policy.
    pub fn new(policy: SchedPolicy) -> Self {
        Self {
            policy: SpinLock::new(policy),
            real_time: {
                let (prio, policy) = match policy {
                    SchedPolicy::RealTime { rt_prio, rt_policy } => (rt_prio.get(), rt_policy),
                    _ => (real_time::RtPrio::MAX, Default::default()),
                };
                real_time::RealTimeAttr::new(prio, policy)
            },
            fair: fair::FairAttr::new(match policy {
                SchedPolicy::Fair(nice) => nice,
                _ => Nice::default(),
            }),
        }
    }

    /// Retrieves the current scheduling policy of the thread.
    pub fn policy(&self) -> SchedPolicy {
        *self.policy.lock()
    }

    /// Updates the scheduling policy of the thread.
    ///
    /// Specifically for real-time policies, if the new policy doesn't
    /// specify a base slice factor for RR, the old one will be kept.
    pub fn set_policy(&self, mut policy: SchedPolicy) {
        let mut guard = self.policy.lock();
        match policy {
            SchedPolicy::RealTime { rt_prio, rt_policy } => {
                self.real_time.update(rt_prio.get(), rt_policy);
            }
            SchedPolicy::Fair(nice) => self.fair.update(nice),
            _ => {}
        }

        // Keep the old base slice factor if the new policy doesn't specify one.
        if let (
            SchedPolicy::RealTime {
                rt_policy:
                    RealTimePolicy::RoundRobin {
                        base_slice_factor: slot,
                    },
                ..
            },
            SchedPolicy::RealTime {
                rt_policy: RealTimePolicy::RoundRobin { base_slice_factor },
                ..
            },
        ) = (*guard, &mut policy)
        {
            *base_slice_factor = slot.or(*base_slice_factor);
        }

        *guard = policy;
    }
}

impl Scheduler for ClassScheduler {
    fn enqueue(&self, task: Arc<Task>, flags: EnqueueFlags) -> Option<CpuId> {
        let thread = task.as_thread()?;

        let (still_in_rq, cpu) = {
            let selected_cpu_id = self.select_cpu(thread.atomic_cpu_affinity());

            if let Err(task_cpu_id) = task.cpu().set_if_is_none(selected_cpu_id) {
                debug_assert!(flags != EnqueueFlags::Spawn);
                (true, task_cpu_id)
            } else {
                (false, selected_cpu_id)
            }
        };

        let mut rq = self.rqs[cpu.as_usize()].disable_irq().lock();

        // Note: call set_if_is_none again to prevent a race condition.
        if still_in_rq && task.cpu().set_if_is_none(cpu).is_err() {
            return None;
        }

        rq.enqueue_thread(thread, Some(flags));
        Some(cpu)
    }

    fn local_mut_rq_with(&self, f: &mut dyn FnMut(&mut dyn LocalRunQueue)) {
        let guard = disable_local();
        let mut lock = self.rqs[guard.current_cpu().as_usize()].lock();
        f(&mut *lock)
    }

    fn local_rq_with(&self, f: &mut dyn FnMut(&dyn LocalRunQueue)) {
        let guard = disable_local();
        f(&*self.rqs[guard.current_cpu().as_usize()].lock())
    }
}

impl ClassScheduler {
    pub fn new() -> Self {
        let stop = stop::StopClassRq::new();
        let class_rq = |cpu| {
            SpinLock::new(PerCpuClassRqSet {
                stop: stop.clone(),
                real_time: real_time::RealTimeClassRq::new(cpu),
                fair: fair::FairClassRq::new(cpu),
                idle: idle::IdleClassRq::new(),
                current: None,
            })
        };
        ClassScheduler {
            rqs: all_cpus().map(class_rq).collect(),
        }
    }

    // TODO: Implement a better algorithm and replace the current naive implementation.
    fn select_cpu(&self, affinity: &AtomicCpuSet) -> CpuId {
        let guard = disable_local();
        let affinity = affinity.load();
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

    fn enqueue_thread(&mut self, thread: &Arc<Thread>, flags: Option<EnqueueFlags>) {
        let attr = thread.sched_attr();

        let cloned = thread.clone();
        match *attr.policy.lock() {
            SchedPolicy::Stop => self.stop.enqueue(cloned, flags),
            SchedPolicy::RealTime { .. } => self.real_time.enqueue(cloned, flags),
            SchedPolicy::Fair(_) => self.fair.enqueue(cloned, flags),
            SchedPolicy::Idle => self.idle.enqueue(cloned, flags),
        };
    }

    fn nr_queued_and_running(&mut self) -> (u32, u32) {
        let queued = self.stop.len() + self.real_time.len() + self.fair.len() + self.idle.len();
        let running = usize::from(self.current.is_some());
        (queued as u32, running as u32)
    }
}

impl LocalRunQueue for PerCpuClassRqSet {
    fn current(&self) -> Option<&Arc<Task>> {
        self.current.as_ref().map(|(task, _)| task)
    }

    fn pick_next_current(&mut self) -> Option<&Arc<Task>> {
        self.pick_next_thread().and_then(|next| {
            let next_task = next.task();
            if let Some((old_task, _)) = self
                .current
                .replace((next_task.clone(), CurrentRuntime::new()))
            {
                if Arc::ptr_eq(&old_task, &next_task) {
                    return None;
                }
                let old = old_task.as_thread().unwrap();
                self.enqueue_thread(old, None);
            }
            self.current.as_ref().map(|(task, _)| task)
        })
    }

    fn update_current(&mut self, flags: UpdateFlags) -> bool {
        if let Some((cur_task, rt)) = &mut self.current
            && let Some(cur) = cur_task.as_thread()
        {
            rt.update();
            let attr = &cur.sched_attr();

            let (current_expired, lookahead) = match &*attr.policy.lock() {
                SchedPolicy::Stop => (self.stop.update_current(rt, attr, flags), 0),
                SchedPolicy::RealTime { .. } => (self.real_time.update_current(rt, attr, flags), 1),
                SchedPolicy::Fair(_) => (self.fair.update_current(rt, attr, flags), 2),
                SchedPolicy::Idle => (self.idle.update_current(rt, attr, flags), 3),
            };

            current_expired
                || (lookahead >= 1 && !self.stop.is_empty())
                || (lookahead >= 2 && !self.real_time.is_empty())
                || (lookahead >= 3 && !self.fair.is_empty())
        } else {
            true
        }
    }

    fn dequeue_current(&mut self) -> Option<Arc<Task>> {
        self.current.take().map(|(cur_task, _)| {
            cur_task.schedule_info().cpu.set_to_none();
            cur_task
        })
    }
}

impl SchedulerStats for ClassScheduler {
    fn nr_queued_and_running(&self) -> (u32, u32) {
        self.rqs.iter().fold((0, 0), |(queued, running), rq| {
            let (q, r) = rq.lock().nr_queued_and_running();
            (queued + q, running + r)
        })
    }
}

impl Default for ClassScheduler {
    fn default() -> Self {
        Self::new()
    }
}
