// SPDX-License-Identifier: MPL-2.0

//! A simple SMP scheduler for kernel-mode tests.
//!
//! Tasks run cooperatively until they exit, block, or explicitly yield. New
//! tasks are placed on the least-loaded run queue allowed by their affinity.
//! A task specifies an affinity by storing a [`CpuSet`] as its task data; tasks
//! with other data may run on any CPU.

use alloc::{
    boxed::Box,
    collections::VecDeque,
    sync::{Arc, Weak},
    vec::Vec,
};

use ostd::{
    cpu::{CpuId, CpuSet, PinCurrentCpu, num_cpus},
    sync::SpinLock,
    task::{
        Task, disable_preempt,
        scheduler::{self, EnqueueFlags, LocalRunQueue, Scheduler, UpdateFlags},
    },
    util::id_set::Id,
};
use spin::Once;

static SCHEDULER: Once<&'static BatchScheduler> = Once::new();

/// Installs the kernel-test scheduler.
pub(super) fn init() {
    let scheduler = *SCHEDULER.call_once(|| Box::leak(Box::new(BatchScheduler::new())));
    scheduler::inject_scheduler(scheduler);
}

/// Waits in an AP's boot context until the scheduler has assigned it work.
pub(super) fn wait_for_runnable() {
    let scheduler = SCHEDULER.get().unwrap();
    while !scheduler.has_local_runnable() {
        core::hint::spin_loop();
    }
}

struct BatchScheduler {
    run_queues: Vec<SpinLock<BatchRunQueue>>,
    assignments: SpinLock<Vec<TaskAssignment>>,
}

struct TaskAssignment {
    task: Weak<Task>,
    cpu: CpuId,
}

impl BatchScheduler {
    fn new() -> Self {
        let run_queues = (0..num_cpus())
            .map(|_| SpinLock::new(BatchRunQueue::new()))
            .collect();
        Self {
            run_queues,
            assignments: SpinLock::new(Vec::new()),
        }
    }

    fn has_local_runnable(&self) -> bool {
        let preempt_guard = disable_preempt();
        let run_queue = self.run_queues[preempt_guard.current_cpu().as_usize()]
            .disable_irq()
            .lock();
        !run_queue.is_empty()
    }

    fn assigned_cpu(&self, task: &Arc<Task>) -> CpuId {
        let task_weak = Arc::downgrade(task);
        let mut assignments = self.assignments.disable_irq().lock();
        assignments.retain(|assignment| assignment.task.strong_count() > 0);
        // Keep wakeups on the initial CPU. The task may be woken while that
        // CPU still holds its run queue lock to switch the task out.
        if let Some(assignment) = assignments
            .iter()
            .find(|assignment| Weak::ptr_eq(&assignment.task, &task_weak))
        {
            return assignment.cpu;
        }

        let default_affinity = CpuSet::new_full();
        let affinity = task
            .data()
            .downcast_ref::<CpuSet>()
            .unwrap_or(&default_affinity);

        let selected_cpu = affinity
            .iter()
            .min_by_key(|cpu| self.run_queues[cpu.as_usize()].disable_irq().lock().len())
            .expect("a task's CPU affinity must not be empty");
        assignments.push(TaskAssignment {
            task: task_weak,
            cpu: selected_cpu,
        });
        selected_cpu
    }
}

impl Scheduler for BatchScheduler {
    fn enqueue(&self, runnable: Arc<Task>, flags: EnqueueFlags) -> Option<CpuId> {
        let target_cpu = self.assigned_cpu(&runnable);
        let is_already_enqueued =
            if let Err(previous_cpu) = runnable.schedule_info().cpu.set_if_is_none(target_cpu) {
                debug_assert!(flags != EnqueueFlags::Spawn);
                debug_assert_eq!(previous_cpu, target_cpu);
                true
            } else {
                false
            };

        let mut run_queue = self.run_queues[target_cpu.as_usize()].disable_irq().lock();
        if is_already_enqueued
            && runnable
                .schedule_info()
                .cpu
                .set_if_is_none(target_cpu)
                .is_err()
        {
            return None;
        }
        run_queue.queue.push_back(runnable);

        // Kernel tests and their helper tasks are cooperatively scheduled.
        None
    }

    fn local_rq_with(&self, f: &mut dyn FnMut(&dyn LocalRunQueue)) {
        let preempt_guard = disable_preempt();
        let current_cpu_index = preempt_guard.current_cpu().as_usize();
        let run_queue: &BatchRunQueue = &self.run_queues[current_cpu_index].disable_irq().lock();
        f(run_queue);
    }

    fn mut_local_rq_with(&self, f: &mut dyn FnMut(&mut dyn LocalRunQueue)) {
        let preempt_guard = disable_preempt();
        let current_cpu_index = preempt_guard.current_cpu().as_usize();
        let run_queue: &mut BatchRunQueue =
            &mut self.run_queues[current_cpu_index].disable_irq().lock();
        f(run_queue);
    }
}

struct BatchRunQueue {
    current: Option<Arc<Task>>,
    queue: VecDeque<Arc<Task>>,
}

impl BatchRunQueue {
    const fn new() -> Self {
        Self {
            current: None,
            queue: VecDeque::new(),
        }
    }

    fn is_empty(&self) -> bool {
        self.current.is_none() && self.queue.is_empty()
    }

    fn len(&self) -> usize {
        self.queue.len() + usize::from(self.current.is_some())
    }
}

impl LocalRunQueue for BatchRunQueue {
    fn current(&self) -> Option<&Arc<Task>> {
        self.current.as_ref()
    }

    fn update_current(&mut self, flags: UpdateFlags) -> bool {
        match flags {
            UpdateFlags::Tick => false,
            UpdateFlags::Wait | UpdateFlags::Yield | UpdateFlags::Exit => !self.queue.is_empty(),
        }
    }

    fn try_pick_next(&mut self) -> Option<&Arc<Task>> {
        let next_task = self.queue.pop_front()?;
        if let Some(previous_task) = self.current.replace(next_task) {
            self.queue.push_back(previous_task);
        }
        self.current.as_ref()
    }

    fn dequeue_current(&mut self) -> Option<Arc<Task>> {
        self.current
            .take()
            .inspect(|task| task.schedule_info().cpu.set_to_none())
    }
}

#[cfg(ktest)]
mod tests {
    use alloc::sync::Arc;
    use core::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};

    use ostd::{
        cpu::{CpuId, CpuSet, num_cpus},
        prelude::ktest,
        sync::{SpinLock, WaitQueue},
        task::{Task, TaskOptions},
    };

    #[ktest]
    fn runs_tests_in_task_context() {
        assert!(Task::current().is_some());
    }

    #[ktest]
    #[serial]
    fn balances_tasks_across_cpus() {
        const TASKS_PER_CPU: usize = 2;

        let nr_cpus = num_cpus();
        let nr_tasks = nr_cpus.checked_mul(TASKS_PER_CPU).unwrap();
        let finished = Arc::new(AtomicUsize::new(0));
        let running = Arc::new(AtomicUsize::new(0));
        let used_cpus: Arc<SpinLock<CpuSet>> = Arc::new(SpinLock::new(CpuSet::new_empty()));

        for _ in 0..nr_tasks {
            let finished = finished.clone();
            let running = running.clone();
            let used_cpus = used_cpus.clone();
            TaskOptions::new(move || {
                used_cpus.lock().add(CpuId::current_racy());
                running.fetch_add(1, Ordering::Release);
                while running.load(Ordering::Acquire) < nr_tasks {
                    Task::yield_now();
                }
                finished.fetch_add(1, Ordering::Release);
            })
            .data(())
            .spawn()
            .unwrap();
        }

        while finished.load(Ordering::Acquire) < nr_tasks {
            Task::yield_now();
        }
        assert_eq!(used_cpus.lock().count(), nr_cpus);
    }

    #[ktest]
    #[serial]
    fn honors_task_affinity() {
        if num_cpus() == 1 {
            return;
        }

        let current_cpu = CpuId::current_racy();
        let target_cpu = ostd::cpu::all_cpus()
            .find(|cpu| *cpu != current_cpu)
            .unwrap();
        let actual_cpu = Arc::new(AtomicU32::new(u32::MAX));
        let finished = Arc::new(AtomicBool::new(false));

        let actual_cpu_clone = actual_cpu.clone();
        let finished_clone = finished.clone();
        TaskOptions::new(move || {
            actual_cpu_clone.store(CpuId::current_racy().into(), Ordering::Relaxed);
            finished_clone.store(true, Ordering::Release);
        })
        .data(CpuSet::from(target_cpu))
        .spawn()
        .unwrap();

        while !finished.load(Ordering::Acquire) {
            Task::yield_now();
        }
        assert_eq!(actual_cpu.load(Ordering::Relaxed), u32::from(target_cpu));
    }

    #[ktest]
    #[serial]
    fn keeps_woken_tasks_on_their_assigned_cpu() {
        if num_cpus() == 1 {
            return;
        }

        let target_cpu = ostd::cpu::all_cpus()
            .find(|cpu| *cpu != CpuId::current_racy())
            .unwrap();
        let wait_queue = Arc::new(WaitQueue::new());
        let started = Arc::new(AtomicBool::new(false));
        let proceed = Arc::new(AtomicBool::new(false));
        let finished = Arc::new(AtomicBool::new(false));
        let actual_cpu = Arc::new(AtomicU32::new(u32::MAX));

        let wait_queue_clone = wait_queue.clone();
        let started_clone = started.clone();
        let proceed_clone = proceed.clone();
        let finished_clone = finished.clone();
        let actual_cpu_clone = actual_cpu.clone();
        TaskOptions::new(move || {
            started_clone.store(true, Ordering::Release);
            wait_queue_clone.wait_until(|| proceed_clone.load(Ordering::Acquire).then_some(()));
            actual_cpu_clone.store(CpuId::current_racy().into(), Ordering::Relaxed);
            finished_clone.store(true, Ordering::Release);
        })
        .data(CpuSet::from(target_cpu))
        .spawn()
        .unwrap();

        while !started.load(Ordering::Acquire) {
            Task::yield_now();
        }
        proceed.store(true, Ordering::Release);
        wait_queue.wake_all();
        while !finished.load(Ordering::Acquire) {
            Task::yield_now();
        }
        assert_eq!(actual_cpu.load(Ordering::Relaxed), u32::from(target_cpu));
    }
}
