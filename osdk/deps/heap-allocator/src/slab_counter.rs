// SPDX-License-Identifier: MPL-2.0

//! A fast, SMP-friendly, per-CPU counter.

use core::sync::atomic::{AtomicIsize, Ordering};

use ostd::{
    cpu::{PinCurrentCpu, all_cpus, local::StaticCpuLocal},
    irq,
};

/// A fast, SMP-friendly, per-CPU counter.
///
/// Updating is fast and scalable, but reading is slow and inaccurate because
/// it sums up all per-CPU values.
///
/// Adding on one CPU and subtracting on another may transiently wrap an
/// individual per-CPU value, but the total stays correct since the per-CPU
/// values are summed; a negative total is reported as zero.
pub(crate) struct SlabCounter {
    per_cpu_counter: &'static StaticCpuLocal<AtomicIsize>,
}

impl SlabCounter {
    /// Creates a new counter backed by the given per-CPU storage.
    pub(crate) const fn new(per_cpu_counter: &'static StaticCpuLocal<AtomicIsize>) -> Self {
        Self { per_cpu_counter }
    }

    /// Adds `delta` to the counter on the current CPU.
    pub(crate) fn add(&self, delta: usize) {
        let guard = irq::disable_local();
        self.per_cpu_counter
            .get_on_cpu(guard.current_cpu())
            .fetch_add(delta as isize, Ordering::Relaxed);
    }

    /// Subtracts `delta` from the counter on the current CPU.
    pub(crate) fn sub(&self, delta: usize) {
        let guard = irq::disable_local();
        self.per_cpu_counter
            .get_on_cpu(guard.current_cpu())
            .fetch_sub(delta as isize, Ordering::Relaxed);
    }

    /// Gets the total value summed up across all CPUs.
    pub(crate) fn get(&self) -> usize {
        let mut total: isize = 0;
        for cpu in all_cpus() {
            total =
                total.wrapping_add(self.per_cpu_counter.get_on_cpu(cpu).load(Ordering::Relaxed));
        }
        if total < 0 { 0 } else { total as usize }
    }
}
