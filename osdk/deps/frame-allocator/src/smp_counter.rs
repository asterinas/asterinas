// SPDX-License-Identifier: MPL-2.0

//! A fast and scalable SMP counter.

use ostd::cpu::{all_cpus, local::StaticCpuLocal, CpuId};

use core::sync::atomic::{AtomicIsize, Ordering};

/// Defines a static fast SMP counter.
//
// See `FastSmpCounter` for more details.
#[macro_export]
macro_rules! fast_smp_counter {
    ($(#[$attr:meta])* $vis:vis static $name:ident : usize;) => { paste::paste!{
        ostd::cpu_local! {
            static [< __LOCAL_COUNTER_ $name >]: core::sync::atomic::AtomicIsize
                = core::sync::atomic::AtomicIsize::new(0);
        }

        $(#[$attr])*
        $vis static $name: $crate::smp_counter::FastSmpCounter =
            $crate::smp_counter::FastSmpCounter::new(
                & [< __LOCAL_COUNTER_ $name >],
            );
    }};
}

/// A fast, SMP-friendly, global counter.
///
/// Users should use [`fast_smp_counter!`] macro to define a static counter.
///
/// Updating it is fast and scalable, but reading is slow and inaccurate.
///
/// An alternative is to use a global atomic, but if all CPUs are updating the
/// same atomic, it causes serious contention. This method address it by using
/// per-CPU counters and summing them up when needed.
///
/// If we constantly adds on one CPU and subtracts on another CPU, it may cause
/// the counters to wrap. However it is fine since if you add them together, it
/// will be correct. It will lead to inconsistency or surprising values for a
/// short period of time.
///
/// Nevertheless, if the sum of added value exceeds [`usize::MAX`] the counter
/// will wrap on overflow.
pub struct FastSmpCounter {
    per_cpu_counter: &'static StaticCpuLocal<AtomicIsize>,
}

impl FastSmpCounter {
    /// Creates a new [`FastSmpCounter`] with the given per-CPU counter.
    ///
    /// This function should only be used by the [`fast_smp_counter!`] macro.
    #[doc(hidden)]
    pub const fn new(per_cpu_counter: &'static StaticCpuLocal<AtomicIsize>) -> Self {
        Self { per_cpu_counter }
    }

    /// Adds `a` to the counter on the given CPU.
    pub fn add(&self, on_cpu: CpuId, a: usize) {
        self.per_cpu_counter
            .get_on_cpu(on_cpu)
            .fetch_add(a as isize, Ordering::Relaxed);
    }

    /// Subtracts `a` from the counter on the given CPU.
    pub fn sub(&self, on_cpu: CpuId, a: usize) {
        self.per_cpu_counter
            .get_on_cpu(on_cpu)
            .fetch_sub(a as isize, Ordering::Relaxed);
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
            0
        } else {
            total as usize
        }
    }
}

#[cfg(ktest)]
mod test {
    use ostd::{cpu::PinCurrentCpu, prelude::*, trap};

    #[ktest]
    fn test_per_cpu_counter() {
        fast_smp_counter! {
            /// The total size of free memory.
            pub static FREE_SIZE_COUNTER: usize;
        }

        let guard = trap::irq::disable_local();
        let cur_cpu = guard.current_cpu();
        FREE_SIZE_COUNTER.add(cur_cpu, 10);
        assert_eq!(FREE_SIZE_COUNTER.get(), 10);
        FREE_SIZE_COUNTER.add(cur_cpu, 20);
        assert_eq!(FREE_SIZE_COUNTER.get(), 30);
        FREE_SIZE_COUNTER.sub(cur_cpu, 5);
        assert_eq!(FREE_SIZE_COUNTER.get(), 25);
    }
}
