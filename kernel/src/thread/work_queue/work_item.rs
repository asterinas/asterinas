// SPDX-License-Identifier: MPL-2.0

#![allow(dead_code)]

use core::sync::atomic::{AtomicBool, Ordering};

use intrusive_collections::{intrusive_adapter, LinkedListAtomicLink};
use ostd::cpu::{CpuId, CpuSet};

use crate::prelude::*;

/// A task to be executed by a worker thread.
pub struct WorkItem {
    work_func: Box<dyn Fn() + Send + Sync>,
    cpu_affinity: CpuSet,
    was_pending: AtomicBool,
    link: LinkedListAtomicLink,
}

intrusive_adapter!(pub(super) WorkItemAdapter = Arc<WorkItem>: WorkItem { link: LinkedListAtomicLink });

impl WorkItem {
    pub fn new(work_func: Box<dyn Fn() + Send + Sync>) -> Arc<WorkItem> {
        let cpu_affinity = CpuSet::new_full();
        Arc::new(WorkItem {
            work_func,
            cpu_affinity,
            was_pending: AtomicBool::new(false),
            link: LinkedListAtomicLink::new(),
        })
    }

    pub fn cpu_affinity(&self) -> &CpuSet {
        &self.cpu_affinity
    }

    pub fn cpu_affinity_mut(&mut self) -> &mut CpuSet {
        &mut self.cpu_affinity
    }

    pub(super) fn is_valid_cpu(&self, cpu_id: CpuId) -> bool {
        self.cpu_affinity.contains(cpu_id)
    }

    pub(super) fn set_processing(&self) {
        self.was_pending.store(false, Ordering::Release);
    }

    pub(super) fn set_pending(&self) {
        self.was_pending.store(true, Ordering::Release);
    }

    pub(super) fn is_pending(&self) -> bool {
        self.was_pending.load(Ordering::Acquire)
    }

    pub(super) fn try_pending(&self) -> bool {
        self.was_pending
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
    }

    pub(super) fn call_work_func(&self) {
        self.work_func.call(())
    }
}
