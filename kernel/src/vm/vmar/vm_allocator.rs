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
    use align_ext::AlignExt;
    use ostd::sync::{PreemptDisabled, SpinLock};

    use super::VmAllocator;
    use crate::{
        prelude::*,
        vm::vmar::{ROOT_VMAR_CAP_ADDR, ROOT_VMAR_GROWUP_BASE},
    };

    /// A simple dense allocator for VMAR.
    pub struct SimpleAllocator {
        alloc_last: SpinLock<Vaddr, PreemptDisabled>,
    }

    impl SimpleAllocator {
        pub fn new() -> Self {
            SimpleAllocator {
                alloc_last: SpinLock::new(ROOT_VMAR_GROWUP_BASE),
            }
        }
    }

    impl VmAllocator for SimpleAllocator {
        fn clear(&self) {
            *self.alloc_last.lock() = ROOT_VMAR_GROWUP_BASE;
        }

        fn allocate(&self, size: usize, align: usize) -> Result<usize> {
            let mut alloc_last = self.alloc_last.lock();
            let allocated = alloc_last.align_up(align);
            let new_end = allocated + size;
            if new_end >= ROOT_VMAR_CAP_ADDR {
                return Err(Error::new(Errno::ENOMEM));
            }
            *alloc_last = new_end;
            Ok(allocated)
        }

        fn allocate_fixed(&self, _start: usize, end: usize) {
            let mut alloc_last = self.alloc_last.lock();
            if *alloc_last < end {
                *alloc_last = end;
            }
        }

        fn fork(&self) -> Self {
            SimpleAllocator {
                alloc_last: SpinLock::new(*self.alloc_last.lock()),
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
        vm::vmar::{ROOT_VMAR_CAP_ADDR, ROOT_VMAR_GROWUP_BASE},
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
            let mut cur_base = ROOT_VMAR_GROWUP_BASE;
            let per_cpu_size =
                ((ROOT_VMAR_CAP_ADDR - ROOT_VMAR_GROWUP_BASE) / nr_cpus).align_down(PAGE_SIZE);
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
