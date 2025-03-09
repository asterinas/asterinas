// SPDX-License-Identifier: MPL-2.0

//! Per-CPU allocator for VMAR

use crate::prelude::*;

pub trait VmAllocator {
    fn allocate(&self, size: usize, align: usize) -> Result<usize>;
    fn allocate_fixed(&self, start: usize, end: usize);
    fn fork(&self) -> Self;
    fn clear(&self);
}

#[cfg(not(feature = "dist_vmar_alloc"))]
mod simple_allocator {
    use core::{
        cmp::{max, min},
        ops::Range,
    };

    use align_ext::AlignExt;
    use ostd::sync::{PreemptDisabled, SpinLock};

    use super::VmAllocator;
    use crate::{
        prelude::*,
        vm::vmar::{INIT_STACK_CLEARANCE, ROOT_VMAR_LOWEST_ADDR},
    };

    /// A simple dense allocator for VMAR.
    pub struct SimpleAllocator {
        free_range: SpinLock<Range<Vaddr>, PreemptDisabled>,
    }

    impl SimpleAllocator {
        pub fn new() -> Self {
            SimpleAllocator {
                free_range: SpinLock::new(ROOT_VMAR_LOWEST_ADDR..INIT_STACK_CLEARANCE),
            }
        }
    }

    impl VmAllocator for SimpleAllocator {
        fn clear(&self) {
            *self.free_range.lock() = ROOT_VMAR_LOWEST_ADDR..INIT_STACK_CLEARANCE;
        }

        fn allocate(&self, size: usize, align: usize) -> Result<usize> {
            let mut free_range = self.free_range.lock();
            let allocated = free_range.start.align_up(align);
            let new_end = allocated + size;
            if new_end > free_range.end {
                log::error!(
                    "VMAR allocator: requested size {:#x?} align {:#x?}, available range: {:#x?}",
                    size,
                    align,
                    *free_range
                );
                return Err(Error::new(Errno::ENOMEM));
            }
            *free_range = new_end..free_range.end;
            Ok(allocated)
        }

        fn allocate_fixed(&self, start: usize, end: usize) {
            let mut free_range = self.free_range.lock();
            if end < free_range.start || start > free_range.end {
                return;
            }
            // Grow the nearest boundary (start or end) to the address
            // to be allocated.
            let start_length = start.saturating_sub(free_range.start);
            let end_length = free_range.end.saturating_sub(end);
            if start_length < end_length {
                free_range.start = min(end, free_range.end);
            } else {
                free_range.end = max(start, free_range.start);
            }
        }

        fn fork(&self) -> Self {
            SimpleAllocator {
                free_range: SpinLock::new(self.free_range.lock().clone()),
            }
        }
    }
}

#[cfg(not(feature = "dist_vmar_alloc"))]
pub use simple_allocator::SimpleAllocator;

#[cfg(feature = "dist_vmar_alloc")]
mod per_cpu_allocator {
    use alloc::vec::Vec;
    use core::{
        ops::Range,
        sync::atomic::{AtomicUsize, Ordering},
    };

    use align_ext::AlignExt;
    use ostd::{cpu::PinCurrentCpu, mm::Vaddr, task::disable_preempt};

    use super::VmAllocator;
    use crate::{
        prelude::*,
        vm::vmar::{INIT_STACK_CLEARANCE, ROOT_VMAR_LOWEST_ADDR},
    };

    /// The per-CPU allocator for VMAR.
    pub struct PerCpuAllocator(Vec<PerCpuLast>);

    // Make it cache-line aligned so there is no false sharing.
    #[repr(align(64))]
    struct PerCpuLast {
        range: Range<Vaddr>,
        /// From this address to the cap the new mappings can be allocated.
        alloc_last: AtomicUsize,
    }

    impl PerCpuLast {
        fn alloc(&self, size: usize, align: usize) -> Result<usize> {
            self.alloc_last
                .fetch_update(Ordering::Release, Ordering::Acquire, |last| {
                    let allocated = last.align_up(align);
                    let new_end = allocated + size;
                    if new_end >= self.range.end {
                        None
                    } else {
                        Some(new_end)
                    }
                })
                .map(|last| last.align_up(align))
                .map_err(|_| Error::new(Errno::ENOMEM))
        }
    }

    impl PerCpuAllocator {
        pub fn new() -> Self {
            let nr_cpus = ostd::cpu::num_cpus();
            let mut cur_base = ROOT_VMAR_LOWEST_ADDR;
            let per_cpu_size =
                ((INIT_STACK_CLEARANCE - ROOT_VMAR_LOWEST_ADDR) / nr_cpus).align_down(PAGE_SIZE);
            let mut alloc_bases = Vec::with_capacity(nr_cpus);
            for _ in 0..nr_cpus {
                alloc_bases.push(PerCpuLast {
                    range: cur_base..cur_base + per_cpu_size,
                    alloc_last: AtomicUsize::new(cur_base),
                });
                cur_base += per_cpu_size;
            }
            PerCpuAllocator(alloc_bases)
        }
    }

    impl VmAllocator for PerCpuAllocator {
        fn clear(&self) {
            for i in self.0.iter() {
                i.alloc_last.store(i.range.start, Ordering::Release);
            }
        }

        fn allocate(&self, size: usize, align: usize) -> Result<usize> {
            let preempt_guard = disable_preempt();
            let cur_cpu = preempt_guard.current_cpu();
            let fastpath_res = self.0[cur_cpu.as_usize()].alloc(size, align);

            if fastpath_res.is_ok() {
                return fastpath_res;
            }

            // fastpath failed, try to find a new range
            for percpulast in self.0.iter() {
                let allocated = percpulast.alloc(size, align);
                if allocated.is_ok() {
                    return allocated;
                }
            }

            // all failed
            Err(Error::new(Errno::ENOMEM))
        }

        fn allocate_fixed(&self, start: usize, end: usize) {
            // find the cpu that the size belongs to
            for percpulast in self.0.iter() {
                let range = &percpulast.range;
                if range.contains(&end) {
                    percpulast
                        .alloc_last
                        .fetch_update(Ordering::Release, Ordering::Acquire, |last| {
                            if last < end {
                                Some(end)
                            } else {
                                Some(last)
                            }
                        })
                        .unwrap();
                } else if range.contains(&start) {
                    // contains the start but not the end
                    percpulast.alloc_last.store(range.end, Ordering::Release);
                }
            }
        }

        fn fork(&self) -> Self {
            let mut alloc_bases = Vec::with_capacity(self.0.len());
            for i in self.0.iter() {
                alloc_bases.push(PerCpuLast {
                    range: i.range.clone(),
                    alloc_last: AtomicUsize::new(i.alloc_last.load(Ordering::Acquire)),
                });
            }
            PerCpuAllocator(alloc_bases)
        }
    }
}

#[cfg(feature = "dist_vmar_alloc")]
pub use per_cpu_allocator::PerCpuAllocator;
