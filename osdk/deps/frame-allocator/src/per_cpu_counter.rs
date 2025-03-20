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

/// Defines a static per-CPU counter.
#[macro_export]
macro_rules! per_cpu_counter {
    ($(#[$attr:meta])* $vis:vis static $name:ident : usize;) => { paste::paste!{
        ostd::cpu_local! {
            static [< __LOCAL_COUNTER_ $name >]: core::sync::atomic::AtomicIsize
                = core::sync::atomic::AtomicIsize::new(0);
        }

        #[expect(non_camel_case_types)]
        $vis struct [< __PerCpuCounter_ $name >];
        $(#[$attr])*
        $vis static $name: [< __PerCpuCounter_ $name >] = [< __PerCpuCounter_ $name >];

        impl [< __PerCpuCounter_ $name >] {
            /// Adds `a` to the counter on the given CPU.
            pub fn add(&self, on_cpu: ostd::cpu::CpuId, a: usize) {
                [< __LOCAL_COUNTER_ $name >]
                    .get_on_cpu(on_cpu)
                    .fetch_add(a as isize, core::sync::atomic::Ordering::Relaxed);
            }

            /// Subtracts `a` from the counter on the given CPU.
            pub fn sub(&self, on_cpu: ostd::cpu::CpuId, a: usize) {
                [< __LOCAL_COUNTER_ $name >]
                    .get_on_cpu(on_cpu)
                    .fetch_sub(a as isize, core::sync::atomic::Ordering::Relaxed);
            }

            /// Gets the total counter value.
            ///
            /// This function may be inaccurate since other CPUs may be
            /// updating the counter.
            pub fn get(&self) -> usize {
                let mut total: isize = 0;
                for cpu in ostd::cpu::all_cpus() {
                    total = total.wrapping_add([< __LOCAL_COUNTER_ $name >]
                        .get_on_cpu(cpu).load(core::sync::atomic::Ordering::Relaxed));
                }
                if total < 0 {
                    0
                } else {
                    total as usize
                }
            }
        }
    }};
}

#[cfg(ktest)]
mod test {
    use ostd::{cpu::PinCurrentCpu, prelude::*, trap};

    #[ktest]
    fn test_per_cpu_counter() {
        per_cpu_counter! {
            /// The total size of free memory.
            pub static FREE_SIZE_COUNTER: usize;
        }

        let guard = trap::disable_local();
        let cur_cpu = guard.current_cpu();
        FREE_SIZE_COUNTER.add(cur_cpu, 10);
        assert_eq!(FREE_SIZE_COUNTER.get(), 10);
        FREE_SIZE_COUNTER.add(cur_cpu, 20);
        assert_eq!(FREE_SIZE_COUNTER.get(), 30);
        FREE_SIZE_COUNTER.sub(cur_cpu, 5);
        assert_eq!(FREE_SIZE_COUNTER.get(), 25);
    }
}
