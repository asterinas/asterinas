// SPDX-License-Identifier: MPL-2.0

#![warn(unused)]

use alloc::{boxed::Box, sync::Arc};
use core::{
    fmt,
    sync::atomic::{AtomicU32, Ordering},
};

use ostd::{
    arch::read_tsc as sched_clock,
    cpu::{all_cpus, CpuId, PinCurrentCpu},
    cpu_local,
    sync::SpinLock,
    task::{
        scheduler::{
            info::CommonSchedInfo, inject_scheduler, EnqueueFlags, LocalRunQueue, Scheduler,
            UpdateFlags,
        },
        AtomicCpuId, Task,
    },
    trap::irq::disable_local,
};

use super::{
    nice::Nice,
    stats::{set_stats_from_scheduler, SchedulerStats},
};
use crate::thread::{AsThread, Thread};

mod policy;
mod time;

mod fair;
mod idle;
mod real_time;
mod stop;

use self::policy::{SchedPolicyKind, SchedPolicyState};
pub use self::{
    policy::SchedPolicy,
    real_time::{RealTimePolicy, RealTimePriority},
};

type SchedEntity = (Arc<Task>, Arc<Thread>);

pub fn init() {
    let scheduler = Box::leak(Box::new(ClassScheduler::new()));

    // Inject the scheduler into the ostd for actual scheduling work.
    inject_scheduler(scheduler);

    // Set the scheduler into the system for statistics.
    // We set this after injecting the scheduler into ostd,
    // so that the loadavg statistics are updated after the scheduler is used.
    set_stats_from_scheduler(scheduler);
}

/// Represents the middle layer between scheduling classes and generic scheduler
/// traits. It consists of all the sets of run queues for CPU cores. Other global
/// information may also be stored here.
pub struct ClassScheduler {
    last_chosen_cpu: AtomicCpuId,
}

cpu_local! {
    static RQ: SpinLock<Option<PerCpuClassRqSet>> = SpinLock::new(const { None });

    static RQ_QUEUED: AtomicU32 = AtomicU32::new(0);
    static RQ_RUNNING: AtomicU32 = AtomicU32::new(0);
}

/// Represents the run queue for each CPU core. It stores a list of run queues for
/// scheduling classes in its corresponding CPU core. The current task of this CPU
/// core is also stored in this structure.
struct PerCpuClassRqSet {
    stop: stop::StopClassRq,
    real_time: real_time::RealTimeClassRq,
    fair: fair::FairClassRq,
    idle: idle::IdleClassRq,
    current: Option<(SchedEntity, CurrentRuntime)>,
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
    fn enqueue(&mut self, task: Arc<Task>, flags: Option<EnqueueFlags>);

    /// Returns the number of threads in the run queue.
    fn len(&self) -> usize;

    /// Checks if the run queue is empty.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Picks the next task for running.
    fn pick_next(&mut self) -> Option<Arc<Task>>;

    /// Update the information of the current task.
    fn update_current(&mut self, rt: &CurrentRuntime, attr: &SchedAttr, flags: UpdateFlags)
        -> bool;
}

/// The scheduling attribute for a thread.
///
/// This is used to store the scheduling policy and runtime parameters for each
/// scheduling class.
#[derive(Debug)]
pub struct SchedAttr {
    policy: SchedPolicyState,
    last_cpu: AtomicCpuId,
    real_time: real_time::RealTimeAttr,
    fair: fair::FairAttr,
}

impl SchedAttr {
    /// Constructs a new `SchedAttr` with the given scheduling policy.
    pub fn new(policy: SchedPolicy) -> Self {
        Self {
            policy: SchedPolicyState::new(policy),
            last_cpu: AtomicCpuId::default(),
            real_time: {
                let (prio, policy) = match policy {
                    SchedPolicy::RealTime { rt_prio, rt_policy } => (rt_prio.get(), rt_policy),
                    _ => (real_time::RealTimePriority::MAX.get(), Default::default()),
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
        self.policy.get()
    }

    fn policy_kind(&self) -> SchedPolicyKind {
        self.policy.kind()
    }

    /// Updates the scheduling policy of the thread.
    ///
    /// Specifically for real-time policies, if the new policy doesn't
    /// specify a base slice factor for RR, the old one will be kept.
    pub fn set_policy(&self, policy: SchedPolicy) {
        self.policy.set(policy, |policy| match policy {
            SchedPolicy::RealTime { rt_prio, rt_policy } => {
                self.real_time.update(rt_prio.get(), rt_policy);
            }
            SchedPolicy::Fair(nice) => self.fair.update(nice),
            _ => {}
        });
    }

    pub fn update_policy<T>(&self, f: impl FnOnce(&mut SchedPolicy) -> T) -> T {
        self.policy.update(|policy| {
            let ret = f(policy);
            match *policy {
                SchedPolicy::RealTime { rt_prio, rt_policy } => {
                    self.real_time.update(rt_prio.get(), rt_policy);
                }
                SchedPolicy::Fair(nice) => self.fair.update(nice),
                _ => {}
            }
            ret
        })
    }

    fn last_cpu(&self) -> Option<CpuId> {
        self.last_cpu.get()
    }

    fn set_last_cpu(&self, cpu_id: CpuId) {
        self.last_cpu.set_anyway(cpu_id);
    }
}

impl Scheduler for ClassScheduler {
    fn enqueue(&self, task: Arc<Task>, flags: EnqueueFlags) -> Option<CpuId> {
        let thread = task.as_thread()?.clone();

        let (still_in_rq, cpu) = {
            let selected_cpu_id = self.select_cpu(&thread, flags);

            if let Err(task_cpu_id) = task.cpu().set_if_is_none(selected_cpu_id) {
                debug_assert!(flags != EnqueueFlags::Spawn);
                (true, task_cpu_id)
            } else {
                (false, selected_cpu_id)
            }
        };

        let rq = RQ.get_on_cpu(cpu);
        let mut rq = rq.disable_irq().lock();
        let rq = rq.as_mut().unwrap();

        // Note: call set_if_is_none again to prevent a race condition.
        if still_in_rq && task.cpu().set_if_is_none(cpu).is_err() {
            return None;
        }

        // Preempt if the new task has a higher priority.
        let should_preempt = rq
            .current
            .as_ref()
            .is_none_or(|((_, rq_current_thread), _)| {
                thread.sched_attr().policy() < rq_current_thread.sched_attr().policy()
            });

        thread.sched_attr().set_last_cpu(cpu);
        rq.enqueue_entity((task, thread), Some(flags));

        should_preempt.then_some(cpu)
    }

    fn mut_local_rq_with(&self, f: &mut dyn FnMut(&mut dyn LocalRunQueue)) {
        let guard = disable_local();
        let rq = RQ.get_on_cpu(guard.current_cpu());
        let mut lock = rq.lock();
        f(lock.as_mut().unwrap())
    }

    fn local_rq_with(&self, f: &mut dyn FnMut(&dyn LocalRunQueue)) {
        let guard = disable_local();
        let rq = RQ.get_on_cpu(guard.current_cpu());
        let lock = rq.lock();
        f(lock.as_ref().unwrap())
    }
}

impl ClassScheduler {
    pub fn new() -> Self {
        for cpu in all_cpus() {
            let rq = RQ.get_on_cpu(cpu);
            let mut rq = rq.lock();
            *rq = Some(PerCpuClassRqSet {
                stop: stop::StopClassRq::new(),
                real_time: real_time::RealTimeClassRq::new(cpu),
                fair: fair::FairClassRq::new(cpu),
                idle: idle::IdleClassRq::new(),
                current: None,
            });
        }
        ClassScheduler {
            last_chosen_cpu: AtomicCpuId::default(),
        }
    }

    // TODO: Implement a better algorithm and replace the current naive implementation.
    fn select_cpu(&self, thread: &Thread, flags: EnqueueFlags) -> CpuId {
        if let Some(last_cpu) = thread.sched_attr().last_cpu() {
            return last_cpu;
        }
        debug_assert!(flags == EnqueueFlags::Spawn);
        let guard = disable_local();
        let affinity = thread.atomic_cpu_affinity().load(Ordering::Relaxed);
        let mut selected = guard.current_cpu();
        let mut minimum_load = u32::MAX;
        let last_chosen = match self.last_chosen_cpu.get() {
            Some(cpu) => cpu.as_usize() as isize,
            None => -1,
        };
        // Simulate a round-robin selection starting from the last chosen CPU.
        //
        // It still checks every CPU to find the one with the minimum load, but
        // avoids keeping selecting the same CPU when there are multiple equally
        // idle CPUs.
        let affinity_iter = affinity
            .iter()
            .filter(|&cpu| cpu.as_usize() as isize > last_chosen)
            .chain(
                affinity
                    .iter()
                    .filter(|&cpu| cpu.as_usize() as isize <= last_chosen),
            );
        for candidate in affinity_iter {
            let load = RQ_QUEUED.get_on_cpu(candidate).load(Ordering::Relaxed);
            if load < minimum_load {
                minimum_load = load;
                selected = candidate;
            }
        }
        self.last_chosen_cpu.set_anyway(selected);
        selected
    }
}

impl PerCpuClassRqSet {
    fn pick_next_entity(&mut self) -> Option<SchedEntity> {
        (self.stop.pick_next())
            .or_else(|| self.real_time.pick_next())
            .or_else(|| self.fair.pick_next())
            .or_else(|| self.idle.pick_next())
            .and_then(|task| {
                let thread = task.as_thread()?.clone();
                Some((task, thread))
            })
    }

    fn pick_non_idle(&mut self) -> Option<SchedEntity> {
        (self.stop.pick_next())
            .or_else(|| self.real_time.pick_next())
            .or_else(|| self.fair.pick_next())
            .and_then(|task| {
                let thread = task.as_thread()?.clone();
                Some((task, thread))
            })
    }

    fn enqueue_entity(&mut self, (task, thread): SchedEntity, flags: Option<EnqueueFlags>) {
        match thread.sched_attr().policy_kind() {
            SchedPolicyKind::Stop => self.stop.enqueue(task, flags),
            SchedPolicyKind::RealTime => self.real_time.enqueue(task, flags),
            SchedPolicyKind::Fair => self.fair.enqueue(task, flags),
            SchedPolicyKind::Idle => self.idle.enqueue(task, flags),
        }
        let (queued, running) = self.nr_queued_and_running();
        RQ_QUEUED
            .get_on_cpu(CpuId::current_racy())
            .store(queued, Ordering::Relaxed);
        RQ_RUNNING
            .get_on_cpu(CpuId::current_racy())
            .store(running, Ordering::Relaxed);
    }

    fn nr_queued_and_running(&self) -> (u32, u32) {
        let queued = self.stop.len() + self.real_time.len() + self.fair.len() + self.idle.len();
        let running = usize::from(self.current.is_some());
        (queued as u32, running as u32)
    }
}

impl LocalRunQueue for PerCpuClassRqSet {
    fn current(&self) -> Option<&Arc<Task>> {
        self.current.as_ref().map(|((task, _), _)| task)
    }

    fn try_pick_next(&mut self) -> Option<&Arc<Task>> {
        self.pick_next_entity().and_then(|next| {
            // We guarantee that a task can appear at once in a `PerCpuClassRqSet`. So, the `next` cannot be the same
            // as the current task here.
            if let Some((old, _)) = self.current.replace((next, CurrentRuntime::new())) {
                self.enqueue_entity(old, None);
            }
            self.current.as_ref().map(|((task, _), _)| task)
        })
    }

    fn update_current(&mut self, flags: UpdateFlags) -> bool {
        if let Some(((_, cur), rt)) = &mut self.current {
            rt.update();
            let attr = &cur.sched_attr();

            let (current_expired, lookahead) = match attr.policy_kind() {
                SchedPolicyKind::Stop => (self.stop.update_current(rt, attr, flags), 0),
                SchedPolicyKind::RealTime => (self.real_time.update_current(rt, attr, flags), 1),
                SchedPolicyKind::Fair => (self.fair.update_current(rt, attr, flags), 2),
                SchedPolicyKind::Idle => (self.idle.update_current(rt, attr, flags), 3),
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
        let task = self.current.take().map(|((cur_task, _), _)| {
            cur_task.schedule_info().cpu.set_to_none();
            cur_task
        });

        let (queued, running) = self.nr_queued_and_running();
        RQ_QUEUED
            .get_on_cpu(CpuId::current_racy())
            .store(queued, Ordering::Relaxed);
        RQ_RUNNING
            .get_on_cpu(CpuId::current_racy())
            .store(running, Ordering::Relaxed);

        task
    }
}

impl SchedulerStats for ClassScheduler {
    fn nr_queued_and_running(&self) -> (u32, u32) {
        let mut queued = 0;
        let mut running = 0;
        for cpu in all_cpus() {
            queued += RQ_QUEUED.get_on_cpu(cpu).load(Ordering::Relaxed);
            running += RQ_RUNNING.get_on_cpu(cpu).load(Ordering::Relaxed);
        }
        (queued, running)
    }
}

impl Default for ClassScheduler {
    fn default() -> Self {
        Self::new()
    }
}

/// Steals a runnable task from another CPU core and schedules it on the current CPU core.
///
/// Must be called from an idle thread.
pub(crate) fn steal_a_task() {
    let cur_cpu = CpuId::current_racy(); // Ok because idle tasks are not migratable.
    let mut most_loaded_cpu = None;
    let mut max_load = 0;

    for cpu in all_cpus() {
        if cpu == cur_cpu {
            continue;
        }
        let load = RQ_QUEUED.get_on_cpu(cpu).load(Ordering::Relaxed);
        if load > max_load {
            max_load = load;
            most_loaded_cpu = Some(cpu);
        }
    }

    let irq_guard = disable_local();

    'out: {
        if max_load <= 1 {
            break 'out;
        }
        let Some(target_cpu) = most_loaded_cpu else {
            break 'out;
        };

        // Lock RQs in the order of CPU IDs to prevent deadlocks.
        let our_rq = RQ.get_on_cpu(cur_cpu);
        let our_lock = if cur_cpu.as_usize() < target_cpu.as_usize() {
            Some(our_rq.lock())
        } else {
            None
        };

        let target_rq = RQ.get_on_cpu(target_cpu);
        let Some(mut target_rq) = target_rq.try_lock() else {
            break 'out;
        };

        let task = target_rq.as_mut().unwrap().pick_non_idle();
        if let Some((task, thread)) = task {
            let mut rq = our_lock.unwrap_or_else(|| our_rq.lock());

            task.cpu().set_anyway(cur_cpu);

            let rq = rq.as_mut().unwrap();
            rq.enqueue_entity((task, thread), None);
        }

        break 'out;
    }

    drop(irq_guard);

    Thread::yield_now();
}
