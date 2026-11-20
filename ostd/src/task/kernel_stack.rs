// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::Ordering;

use crate::{
    arch::mm::tlb_flush_addr_range,
    cpu::{AtomicCpuSet, CpuSet, PinCurrentCpu},
    impl_frame_meta_for,
    irq::DisabledLocalIrqGuard,
    mm::{
        FrameAllocOptions, PAGE_SIZE,
        kspace::kvirt_area::KVirtArea,
        page_prop::{CachePolicy, PageFlags, PageProperty, PrivilegedPageFlags},
    },
    prelude::*,
};

/// The kernel stack size of a task, specified in pages.
///
/// By default, we choose a rather large stack size.
/// OSTD users can choose a smaller size by specifying
/// the `OSTD_TASK_STACK_SIZE_IN_PAGES` environment variable
/// at build time.
const STACK_SIZE_IN_PAGES: u32 =
    if let Some(size_str) = option_env!("OSTD_TASK_STACK_SIZE_IN_PAGES") {
        match u32::from_str_radix(size_str, 10) {
            Ok(size) => size,
            Err(_) => panic!(
                "The environment variable `OSTD_TASK_STACK_SIZE_IN_PAGES` \
                    specifies an invalid value"
            ),
        }
    } else {
        DEFAULT_STACK_SIZE_IN_PAGES
    };

/// The default kernel stack size of a task, specified in pages.
const DEFAULT_STACK_SIZE_IN_PAGES: u32 = 128;

const KERNEL_STACK_SIZE: usize = STACK_SIZE_IN_PAGES as usize * PAGE_SIZE;

#[derive(Debug)]
pub(super) struct KernelStack {
    kvirt_area: KVirtArea,
    tlb_coherent: AtomicCpuSet,
    end_vaddr: Vaddr,
}

#[derive(Debug, Default)]
struct KernelStackMeta;

impl_frame_meta_for!(KernelStackMeta);

impl KernelStack {
    /// Generates a kernel stack with guard pages.
    ///
    /// 4 additional pages are allocated and regarded as guard pages, which
    /// should not be accessed.
    //
    // TODO: We map kernel stacks in the kernel virtual areas, which incurs
    // non-negligible TLB and mapping overhead on task creation. This could
    // be improved by caching/reusing kernel stacks with a pool.
    pub(super) fn new_with_guard_page() -> Result<Self> {
        let mut options = FrameAllocOptions::new();
        options.zeroed(false);
        let pages = (0..KERNEL_STACK_SIZE / PAGE_SIZE)
            .map(|_| options.alloc_frame_with(KernelStackMeta))
            .collect::<Result<Vec<_>>>()?;
        let prop = PageProperty {
            flags: PageFlags::RW,
            cache: CachePolicy::Writeback,
            priv_flags: PrivilegedPageFlags::empty(),
        };
        let new_kvirt_area = KVirtArea::map_frames(
            KERNEL_STACK_SIZE + 4 * PAGE_SIZE,
            2 * PAGE_SIZE,
            pages.into_iter(),
            prop,
        );
        let mapped_start = new_kvirt_area.range().start + 2 * PAGE_SIZE;
        let mapped_end = mapped_start + KERNEL_STACK_SIZE;
        Ok(Self {
            kvirt_area: new_kvirt_area,
            tlb_coherent: AtomicCpuSet::new(CpuSet::new_empty()),
            end_vaddr: mapped_end,
        })
    }

    /// Flushes the TLB for the current CPU if necessary.
    pub(super) fn flush_tlb(&self, irq_guard: &DisabledLocalIrqGuard) {
        let cur_cpu = irq_guard.current_cpu();
        if !self.tlb_coherent.contains(cur_cpu, Ordering::Relaxed) {
            tlb_flush_addr_range(&self.kvirt_area.range());
            self.tlb_coherent.add(cur_cpu, Ordering::Relaxed);
        }
    }

    pub(super) fn end_vaddr(&self) -> Vaddr {
        self.end_vaddr
    }
}

#[cfg(ktest)]
mod test {
    use super::{KERNEL_STACK_SIZE, KernelStack, KernelStackMeta};
    use crate::{
        mm::{
            Frame, FrameAllocOptions, PAGE_SIZE,
            kspace::MappedItemRef,
            page_prop::{CachePolicy, PageFlags},
        },
        prelude::*,
        task::disable_preempt,
    };

    #[ktest]
    fn map_noncontiguous_stack_with_guard_pages() {
        let stack_pages = KERNEL_STACK_SIZE / PAGE_SIZE;
        drop(Vec::<Frame<KernelStackMeta>>::with_capacity(stack_pages));

        let mut options = FrameAllocOptions::new();
        options.zeroed(false);

        let guard = disable_preempt();
        let segment = options.alloc_segment(3).unwrap();
        let mut iter = segment.into_iter();
        let left = iter.next().unwrap();
        let mid = iter.next().unwrap();
        let right = iter.next().unwrap();
        assert!(iter.next().is_none());

        let _drained = core::array::from_fn::<_, 4, _>(|_| options.alloc_frame().unwrap());

        let left_pa = left.paddr();
        let mid_pa = mid.paddr();
        let right_pa = right.paddr();
        drop(right);
        drop(left);

        let stack = KernelStack::new_with_guard_page().unwrap();
        let base = stack.kvirt_area.start();
        assert_eq!(stack.kvirt_area.size(), KERNEL_STACK_SIZE + 4 * PAGE_SIZE);
        assert_eq!(stack.end_vaddr(), base + 2 * PAGE_SIZE + KERNEL_STACK_SIZE);

        for guard_index in [0, 1, 2 + stack_pages, 3 + stack_pages] {
            assert!(
                stack
                    .kvirt_area
                    .query(&guard, base + guard_index * PAGE_SIZE)
                    .is_none()
            );
        }

        let mut paddrs = Vec::with_capacity(stack_pages);
        for page_index in 0..stack_pages {
            let addr = base + (2 + page_index) * PAGE_SIZE;
            let MappedItemRef::Tracked(frame, prop) = stack.kvirt_area.query(&guard, addr).unwrap()
            else {
                panic!("Expected a tracked kernel stack frame");
            };
            assert_eq!(prop.flags, PageFlags::RW);
            assert_eq!(prop.cache, CachePolicy::Writeback);
            paddrs.push(frame.paddr());
        }

        assert!(paddrs.contains(&left_pa));
        assert!(paddrs.contains(&right_pa));
        assert!(!paddrs.contains(&mid_pa));
        assert!(paddrs.windows(2).any(|pair| pair[1] != pair[0] + PAGE_SIZE));
    }
}
