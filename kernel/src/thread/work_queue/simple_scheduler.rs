// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Weak;

use super::worker_pool::{WorkerPool, WorkerScheduler};

/// SimpleScheduler is the simplest scheduling implementation.
/// Only when there is a liveness problem in the workerpool, increase the workers,
/// set the upper limit of the workers, and do not actively reduce the workers.
/// And it only adds one worker at a time for each scheduling.
pub struct SimpleScheduler {
    worker_pool: Weak<WorkerPool>,
}

impl SimpleScheduler {
    pub fn new(worker_pool: Weak<WorkerPool>) -> Self {
        Self { worker_pool }
    }
}

const WORKER_LIMIT: u16 = 16;

impl WorkerScheduler for SimpleScheduler {
    fn schedule(&self) {
        let worker_pool = self.worker_pool.upgrade().unwrap();
        for cpu_id in worker_pool.cpu_set().iter() {
            if !worker_pool.heartbeat(cpu_id)
                && worker_pool.has_pending_work_items(cpu_id)
                && !worker_pool.wake_worker(cpu_id)
                && worker_pool.num_workers(cpu_id) < WORKER_LIMIT
            {
                worker_pool.add_worker(cpu_id);
            }
        }
    }
}
