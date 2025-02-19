// SPDX-License-Identifier: MPL-2.0

//! A per-CPU counter for the total size of free memory.
//!
//! If all CPUs are updating the same counter, it causes serious contention.
//! We address it by using per-CPU counters and summing them up when needed.
//!
//! Updating is fast and scalable, but reading is slow and inaccurate.
//!
//! If we constantly allocates on one CPU and deallocates on another CPU,
//! it may cause the counters to wrap. However it is fine since if you
//! add them together, it will be correct. It will lead to inconsistency
//! or surprising values for a short period of time.

use core::sync::atomic::{AtomicIsize, Ordering};

use ostd::{cpu::all_cpus, cpu_local, trap::DisabledLocalIrqGuard};

cpu_local! {
    static FREE_SIZE: AtomicIsize = AtomicIsize::new(0);
}

/// Adds the given size to a global total free size.
pub(super) fn add_free_size(irq_guard: &DisabledLocalIrqGuard, size: usize) {
    FREE_SIZE
        .get_with(irq_guard)
        .fetch_add(size as isize, Ordering::Relaxed);
}

/// Subtracts the given size from a global total free size.
pub(super) fn sub_free_size(irq_guard: &DisabledLocalIrqGuard, size: usize) {
    FREE_SIZE
        .get_with(irq_guard)
        .fetch_sub(size as isize, Ordering::Relaxed);
}

/// Reads the total size of free memory.
///
/// This function is not atomic and may be inaccurate since other CPUs may be
/// updating the counter while we are reading it.
pub(super) fn read_total_free_size() -> usize {
    let mut total: isize = 0;
    for cpu in all_cpus() {
        total = total.wrapping_add(FREE_SIZE.get_on_cpu(cpu).load(Ordering::Relaxed));
    }
    if total < 0 {
        0
    } else {
        total as usize
    }
}
