// SPDX-License-Identifier: MPL-2.0

//! A fast and scalable per-CPU counter.

use core::sync::atomic::{AtomicIsize, Ordering};

use osdk_heap_allocator::{alloc_cpu_local, dealloc_cpu_local};
use ostd::cpu::{
    all_cpus,
    local::{CpuLocal, DynamicStorage},
    CpuId,
};

/// A fast, SMP-friendly, dynamically allocated, per-CPU counter.
///
/// Updating it is fast and scalable, but reading is slow and inaccurate.
///
/// See [`osdk_frame_allocator::fast_smp_counter`] for more details.
pub struct PerCpuCounter {
    per_cpu_counter: Option<CpuLocal<AtomicIsize, DynamicStorage<AtomicIsize>>>,
}

impl PerCpuCounter {
    pub fn new() -> Self {
        Self {
            per_cpu_counter: Some(alloc_cpu_local(|_| AtomicIsize::new(0)).unwrap()),
        }
    }

    fn per_cpu_counter(&self) -> &CpuLocal<AtomicIsize, DynamicStorage<AtomicIsize>> {
        self.per_cpu_counter.as_ref().unwrap()
    }

    /// Adds `increment` to the counter on the given CPU.
    pub fn add(&self, on_cpu: CpuId, increment: usize) {
        self.per_cpu_counter()
            .get_on_cpu(on_cpu)
            .fetch_add(increment as isize, Ordering::Relaxed);
    }

    /// Subtracts `decrement` from the counter on the given CPU.
    pub fn sub(&self, on_cpu: CpuId, decrement: usize) {
        self.per_cpu_counter()
            .get_on_cpu(on_cpu)
            .fetch_sub(decrement as isize, Ordering::Relaxed);
    }

    /// Gets the total counter value.
    ///
    /// This function may be inaccurate since other CPUs may be
    /// updating the counter.
    pub fn get(&self) -> usize {
        let mut total: isize = 0;
        for cpu in all_cpus() {
            total = total.wrapping_add(
                self.per_cpu_counter()
                    .get_on_cpu(cpu)
                    .load(Ordering::Relaxed),
            );
        }
        if total < 0 {
            0
        } else {
            total as usize
        }
    }
}

impl Drop for PerCpuCounter {
    fn drop(&mut self) {
        dealloc_cpu_local(self.per_cpu_counter.take().unwrap());
    }
}
