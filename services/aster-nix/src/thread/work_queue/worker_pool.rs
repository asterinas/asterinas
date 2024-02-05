// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicBool, Ordering};

use super::{simple_scheduler::SimpleScheduler, worker::Worker, WorkItem, WorkPriority, WorkQueue};
use crate::prelude::*;
use crate::thread::kernel_thread::{KernelThreadExt, ThreadOptions};
use crate::Thread;
use aster_frame::cpu::CpuSet;
use aster_frame::sync::WaitQueue;
use aster_frame::task::Priority;

/// A pool of workers.
///
/// The `WorkerPool` maintains workers created from different CPUs, while clustering workers
/// from the same CPU into a `LocalWorkerPool` for better management.
pub struct WorkerPool {
    local_pools: Vec<Arc<LocalWorkerPool>>,
    /// Monitor invokes `schedule()` in WorkerScheduler to determine whether there is a need for
    /// adding or removing workers.
    monitor: Arc<Monitor>,
    priority: WorkPriority,
    cpu_set: CpuSet,
    scheduler: Arc<dyn WorkerScheduler>,
    work_queues: SpinLock<Vec<Arc<WorkQueue>>>,
}

/// A set of workers for a specific CPU.
pub struct LocalWorkerPool {
    cpu_id: u32,
    idle_wait_queue: WaitQueue,
    parent: Weak<WorkerPool>,
    /// A liveness check for LocalWorkerPool. The monitor periodically clears heartbeat,
    /// and when a worker completes an item, it will be set to indicate that there is still
    /// an active worker. If there is no heartbeats and there are still pending work items,
    /// it suggests that more workers are needed.
    heartbeat: AtomicBool,
    workers: SpinLock<VecDeque<Arc<Worker>>>,
}

/// Schedule `Workers` for a `WorkerPool`.
///
/// Having an excessive number of Workers in WorkerPool may result in wastage of system
/// resources, while a shortage of workers may lead to longer response time for workitems.
/// A well-designed WorkerScheduler must strike a balance between resource utilization and response time.
pub trait WorkerScheduler: Sync + Send {
    /// Schedule workers in a worker pool. This needs to solve two problems: when to increase or decrease
    /// workers, and how to add or remove workers to keep the number of workers in a reasonable range.
    fn schedule(&self);
}

/// The `Monitor` is responsible for monitoring the `WorkerPool` for scheduling needs.
/// Currently, it only performs a liveness check, and attempts to schedule when no workers
/// are found processing in the pool.
pub struct Monitor {
    worker_pool: Weak<WorkerPool>,
    bound_thread: Arc<Thread>,
}

impl LocalWorkerPool {
    fn new(worker_pool: Weak<WorkerPool>, cpu_id: u32) -> Self {
        LocalWorkerPool {
            cpu_id,
            idle_wait_queue: WaitQueue::new(),
            parent: worker_pool,
            heartbeat: AtomicBool::new(false),
            workers: SpinLock::new(VecDeque::new()),
        }
    }

    fn add_worker(&self) {
        let worker = Worker::new(self.parent.clone(), self.cpu_id);
        self.workers.lock_irq_disabled().push_back(worker.clone());
        worker.run();
    }

    fn remove_worker(&self) {
        let mut workers = self.workers.lock_irq_disabled();
        for (index, worker) in workers.iter().enumerate() {
            if worker.is_idle() {
                worker.destroy();
                workers.remove(index);
                break;
            }
        }
    }

    fn wake_worker(&self) -> bool {
        if !self.idle_wait_queue.is_empty() {
            self.idle_wait_queue.wake_one();
            return true;
        }
        false
    }

    fn has_pending_work_items(&self) -> bool {
        self.parent
            .upgrade()
            .unwrap()
            .has_pending_work_items(self.cpu_id)
    }

    fn heartbeat(&self) -> bool {
        self.heartbeat.load(Ordering::Acquire)
    }

    fn set_heartbeat(&self, heartbeat: bool) {
        self.heartbeat.store(heartbeat, Ordering::Release);
    }

    fn idle_current_worker(&self, worker: Arc<Worker>) {
        self.idle_wait_queue
            .wait_until(|| (worker.is_destroying() || self.has_pending_work_items()).then_some(0));
    }

    fn destroy_all_workers(&self) {
        for worker in self.workers.lock_irq_disabled().iter() {
            worker.destroy();
        }
        self.idle_wait_queue.wake_all();
    }
}

impl WorkerPool {
    pub fn new(priority: WorkPriority, cpu_set: CpuSet) -> Arc<Self> {
        Arc::new_cyclic(|pool_ref| {
            let mut local_pools = Vec::new();
            for cpu_id in cpu_set.iter() {
                local_pools.push(Arc::new(LocalWorkerPool::new(
                    pool_ref.clone(),
                    cpu_id as u32,
                )));
            }
            WorkerPool {
                local_pools,
                monitor: Monitor::new(pool_ref.clone()),
                priority,
                cpu_set,
                scheduler: Arc::new(SimpleScheduler::new(pool_ref.clone())),
                work_queues: SpinLock::new(Vec::new()),
            }
        })
    }

    pub fn run(&self) {
        self.monitor.run();
    }

    pub fn assign_work_queue(&self, work_queue: Arc<WorkQueue>) {
        self.work_queues.lock_irq_disabled().push(work_queue);
    }

    pub fn has_pending_work_items(&self, request_cpu: u32) -> bool {
        self.work_queues
            .lock_irq_disabled()
            .iter()
            .any(|work_queue| work_queue.has_pending_work_items(request_cpu))
    }

    pub fn schedule(&self) {
        self.scheduler.schedule();
    }

    pub fn num_workers(&self, cpu_id: u32) -> u16 {
        self.local_pool(cpu_id).workers.lock_irq_disabled().len() as u16
    }

    pub fn cpu_set(&self) -> &CpuSet {
        &self.cpu_set
    }

    pub(super) fn fetch_pending_work_item(&self, request_cpu: u32) -> Option<Arc<WorkItem>> {
        for work_queue in self.work_queues.lock_irq_disabled().iter() {
            let item = work_queue.dequeue(request_cpu);
            if item.is_some() {
                return item;
            }
        }
        None
    }

    fn local_pool(&self, cpu_id: u32) -> &Arc<LocalWorkerPool> {
        self.local_pools
            .iter()
            .find(|local_pool: &&Arc<LocalWorkerPool>| local_pool.cpu_id == cpu_id)
            .unwrap()
    }

    pub(super) fn wake_worker(&self, cpu_id: u32) -> bool {
        self.local_pool(cpu_id).wake_worker()
    }

    pub(super) fn add_worker(&self, cpu_id: u32) {
        self.local_pool(cpu_id).add_worker();
    }

    pub(super) fn remove_worker(&self, cpu_id: u32) {
        self.local_pool(cpu_id).remove_worker();
    }

    pub(super) fn is_high_priority(&self) -> bool {
        self.priority == WorkPriority::High
    }

    pub(super) fn heartbeat(&self, cpu_id: u32) -> bool {
        self.local_pool(cpu_id).heartbeat()
    }

    pub(super) fn set_heartbeat(&self, cpu_id: u32, heartbeat: bool) {
        self.local_pool(cpu_id).set_heartbeat(heartbeat)
    }

    pub(super) fn idle_current_worker(&self, cpu_id: u32, worker: Arc<Worker>) {
        self.local_pool(cpu_id).idle_current_worker(worker);
    }
}

impl Drop for WorkerPool {
    fn drop(&mut self) {
        for local_pool in self.local_pools.iter() {
            local_pool.destroy_all_workers();
        }
    }
}

impl Monitor {
    pub fn new(worker_pool: Weak<WorkerPool>) -> Arc<Self> {
        Arc::new_cyclic(|monitor_ref| {
            let weal_monitor = monitor_ref.clone();
            let task_fn = Box::new(move || {
                let current_monitor: Arc<Monitor> = weal_monitor.upgrade().unwrap();
                current_monitor.run_monitor_loop();
            });
            let cpu_affinity = CpuSet::new_full();
            let bound_thread = Thread::new_kernel_thread(
                ThreadOptions::new(task_fn)
                    .cpu_affinity(cpu_affinity)
                    .priority(Priority::normal()),
            );
            Self {
                worker_pool,
                bound_thread,
            }
        })
    }

    pub fn run(&self) {
        self.bound_thread.run();
    }

    fn run_monitor_loop(self: &Arc<Self>) {
        loop {
            let worker_pool = self.worker_pool.upgrade();
            let Some(worker_pool) = worker_pool else {
                break;
            };
            worker_pool.schedule();
            for local_pool in worker_pool.local_pools.iter() {
                local_pool.set_heartbeat(false);
            }
            Thread::yield_now();
        }
    }
}
