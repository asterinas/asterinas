// SPDX-License-Identifier: MPL-2.0

//! A fast and scalable per-CPU counter.

use core::sync::atomic::{AtomicIsize, Ordering};

use osdk_heap_allocator::{alloc_cpu_local, CpuLocalBox};
use ostd::cpu::{all_cpus, CpuId};

/// A fast, SMP-friendly, dynamically allocated, per-CPU counter.
///
/// Updating it is fast and scalable, but reading is slow and inaccurate.
///
// TODO: Reuse the code from [`osdk_frame_allocator::fast_smp_counter`],
// which may need to extract that code into a separate crate that needs
// to be published. Do that after we somehow stabilize the per-CPU counter.
pub struct PerCpuCounter {
    per_cpu_counter: CpuLocalBox<AtomicIsize>,
}

impl PerCpuCounter {
    /// Creates a new, zero-valued per-CPU counter.
    pub fn new() -> Self {
        Self {
            per_cpu_counter: alloc_cpu_local(|_| AtomicIsize::new(0)).unwrap(),
        }
    }

    /// Adds `increment` to the counter on the given CPU.
    pub fn add(&self, on_cpu: CpuId, increment: isize) {
        self.per_cpu_counter
            .get_on_cpu(on_cpu)
            .fetch_add(increment, Ordering::Relaxed);
    }

    /// Gets the total counter value.
    ///
    /// This function may be inaccurate since other CPUs may be
    /// updating the counter.
    pub fn get(&self) -> usize {
        let mut total: isize = 0;
        for cpu in all_cpus() {
            total =
                total.wrapping_add(self.per_cpu_counter.get_on_cpu(cpu).load(Ordering::Relaxed));
        }
        if total < 0 {
            // The counter is unsigned. But an observer may see a negative
            // value due to race conditions. We return zero if it happens.
            0
        } else {
            total as usize
        }
    }
}
