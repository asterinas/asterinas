// SPDX-License-Identifier: MPL-2.0

#![allow(dead_code)]

use ostd::{
    cpu::{CpuId, CpuSet},
    task::Task,
};

use super::worker_pool::WorkerPool;
use crate::{
    prelude::*,
    sched::priority::{Priority, PriorityRange},
    thread::{kernel_thread::ThreadOptions, AsThread},
};

/// A worker thread. A `Worker` will attempt to retrieve unfinished
/// work items from its corresponding `WorkerPool`. If there are none,
/// it will go to sleep and be rescheduled when a new work item is
/// added to the `WorkerPool`.
pub(super) struct Worker {
    worker_pool: Weak<WorkerPool>,
    bound_task: Arc<Task>,
    bound_cpu: CpuId,
    inner: SpinLock<WorkerInner>,
}

struct WorkerInner {
    worker_status: WorkerStatus,
}

#[derive(PartialEq)]
enum WorkerStatus {
    Idle,
    Running,
    Exited,
    /// This state only occurs when destructing the `WorkerPool`,
    /// where workers will exit after processing the remaining work items.
    Destroying,
}

impl Worker {
    /// Creates a new `Worker` to the given `worker_pool`.
    pub(super) fn new(worker_pool: Weak<WorkerPool>, bound_cpu: CpuId) -> Arc<Self> {
        Arc::new_cyclic(|worker_ref| {
            let weal_worker = worker_ref.clone();
            let task_fn = Box::new(move || {
                let current_worker: Arc<Worker> = weal_worker.upgrade().unwrap();
                current_worker.run_worker_loop();
            });
            let mut cpu_affinity = CpuSet::new_empty();
            cpu_affinity.add(bound_cpu);
            let mut priority = Priority::default();
            if worker_pool.upgrade().unwrap().is_high_priority() {
                // FIXME: remove the use of real-time priority.
                priority = Priority::new(PriorityRange::new(0));
            }
            let bound_task = ThreadOptions::new(task_fn)
                .cpu_affinity(cpu_affinity)
                .priority(priority)
                .build();
            Self {
                worker_pool,
                bound_task,
                bound_cpu,
                inner: SpinLock::new(WorkerInner {
                    worker_status: WorkerStatus::Running,
                }),
            }
        })
    }

    pub(super) fn run(&self) {
        let thread = self.bound_task.as_thread().unwrap();
        thread.run();
    }

    /// The thread function bound to normal workers.
    /// It pulls a work item from the work queue and sleeps if there is no more pending items.
    fn run_worker_loop(self: &Arc<Self>) {
        loop {
            let worker_pool = self.worker_pool.upgrade();
            let Some(worker_pool) = worker_pool else {
                break;
            };
            if let Some(work_item) = worker_pool.fetch_pending_work_item(self.bound_cpu) {
                work_item.set_processing();
                work_item.call_work_func();
                worker_pool.set_heartbeat(self.bound_cpu, true);
            } else {
                if self.is_destroying() {
                    break;
                }
                self.inner.disable_irq().lock().worker_status = WorkerStatus::Idle;
                worker_pool.idle_current_worker(self.bound_cpu, self.clone());
                if !self.is_destroying() {
                    self.inner.disable_irq().lock().worker_status = WorkerStatus::Running;
                }
            }
        }
        self.exit();
    }

    pub(super) fn bound_task(&self) -> &Arc<Task> {
        &self.bound_task
    }

    pub(super) fn is_idle(&self) -> bool {
        self.inner.disable_irq().lock().worker_status == WorkerStatus::Idle
    }

    pub(super) fn is_destroying(&self) -> bool {
        self.inner.disable_irq().lock().worker_status == WorkerStatus::Destroying
    }

    pub(super) fn destroy(&self) {
        self.inner.disable_irq().lock().worker_status = WorkerStatus::Destroying;
    }

    fn exit(&self) {
        self.inner.disable_irq().lock().worker_status = WorkerStatus::Exited;
    }

    pub(super) fn is_exit(&self) -> bool {
        self.inner.disable_irq().lock().worker_status == WorkerStatus::Exited
    }
}
