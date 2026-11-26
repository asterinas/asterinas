// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::Ordering;

use crate::{
    Error,
    arch::mm::tlb_flush_addr_range,
    cpu::{AtomicCpuSet, CpuSet, PinCurrentCpu},
    impl_frame_meta_for,
    irq::DisabledLocalIrqGuard,
    mm::{
        Frame, FrameAllocOptions, PAGE_SIZE,
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

/// Guard-page span placed on each side of a mapped kernel stack.
const GUARD_SIZE: usize = 2 * PAGE_SIZE;

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
        Self::try_new_with_guard_page(KERNEL_STACK_SIZE / PAGE_SIZE, || {
            options.alloc_frame_with(KernelStackMeta)
        })
    }

    fn try_new_with_guard_page(
        stack_pages: usize,
        allocate_frame_fn: impl FnMut() -> Result<Frame<KernelStackMeta>>,
    ) -> Result<Self> {
        if stack_pages == 0 {
            return Err(Error::InvalidArgs);
        }
        let stack_size = stack_pages
            .checked_mul(PAGE_SIZE)
            .ok_or(Error::InvalidArgs)?;
        let area_size = stack_size
            .checked_add(2 * GUARD_SIZE)
            .ok_or(Error::InvalidArgs)?;

        // Allocate frames before taking kernel page table locks in
        // `KVirtArea::map_frames`, avoiding a multi-page contiguous segment that
        // virtual mapping does not require.
        let mut chunks = allocate_stack_frames(stack_pages, allocate_frame_fn)?;

        let prop = PageProperty {
            flags: PageFlags::RW,
            cache: CachePolicy::Writeback,
            priv_flags: PrivilegedPageFlags::empty(),
        };
        let new_kvirt_area = KVirtArea::map_frames(
            area_size,
            GUARD_SIZE,
            chunks.iter_mut().flat_map(|chunk| chunk.drain(..)),
            prop,
        );
        let mapped_start = new_kvirt_area.range().start + GUARD_SIZE;
        let mapped_end = mapped_start + stack_size;
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

// Match the global heap's `PAGE_SIZE / 2` slab ceiling so per-chunk handle
// buffers come from single-page slabs rather than `HeapSlot::alloc_large`.
const MAX_SLAB_BYTES: usize = PAGE_SIZE / 2;
const FRAMES_PER_CHUNK: usize = MAX_SLAB_BYTES / size_of::<Frame<KernelStackMeta>>();

fn allocate_stack_frames(
    stack_pages: usize,
    mut allocate_frame_fn: impl FnMut() -> Result<Frame<KernelStackMeta>>,
) -> Result<Vec<Vec<Frame<KernelStackMeta>>>> {
    let mut chunks = Vec::new();
    chunks
        .try_reserve_exact(stack_pages.div_ceil(FRAMES_PER_CHUNK))
        .map_err(|_| Error::NoMemory)?;
    let mut remaining = stack_pages;
    while remaining > 0 {
        let chunk_len = remaining.min(FRAMES_PER_CHUNK);
        let mut chunk = Vec::new();
        chunk
            .try_reserve_exact(chunk_len)
            .map_err(|_| Error::NoMemory)?;
        for _ in 0..chunk_len {
            chunk.push(allocate_frame_fn()?);
        }
        chunks.push(chunk);
        remaining -= chunk_len;
    }

    Ok(chunks)
}

#[cfg(ktest)]
mod test {
    use super::{KernelStack, KernelStackMeta};
    use crate::{
        Error, irq,
        mm::{
            self, FrameAllocOptions,
            kspace::MappedItemRef,
            page_prop::{CachePolicy, PageFlags},
        },
        prelude::*,
    };

    #[ktest]
    fn reject_zero_page_stack_and_propagate_frame_shortage() {
        // Preserve `InvalidArgs` on an empty stack and ensure a mid-allocation
        // shortage returns `NoMemory` while dropping already-staged frames.
        let zero_err = KernelStack::try_new_with_guard_page(0, || {
            panic!("zero-page stack must not allocate frames");
        })
        .unwrap_err();
        assert_eq!(zero_err, Error::InvalidArgs);

        let mut options = FrameAllocOptions::new();
        options.zeroed(false);
        let shared_frame = options.alloc_frame_with(KernelStackMeta).unwrap();

        // Span two staging chunks (`FRAMES_PER_CHUNK + 1` succeeded handles
        // followed by `NoMemory`) and verify all cloned handles are dropped.
        let failing_call = super::FRAMES_PER_CHUNK + 2;
        let mut nr_calls = 0;
        let oom_err = KernelStack::try_new_with_guard_page(failing_call, || {
            nr_calls += 1;
            if nr_calls < failing_call {
                Ok(shared_frame.clone())
            } else {
                Err(Error::NoMemory)
            }
        })
        .unwrap_err();
        assert_eq!(oom_err, Error::NoMemory);
        assert_eq!(nr_calls, failing_call);
        assert_eq!(shared_frame.reference_count(), 1);
    }

    #[ktest]
    fn map_noncontiguous_stack_with_guard_pages() {
        let mut options = FrameAllocOptions::new();
        options.zeroed(false);

        // Split a 3-frame segment and keep `_mid` alive so `left` and `right`
        // have a guaranteed physical gap between them.
        let segment = options.alloc_segment_with(3, |_| KernelStackMeta).unwrap();
        let mut iter = segment.into_iter();
        let left = iter.next().unwrap();
        let _mid = iter.next().unwrap();
        let right = iter.next().unwrap();
        assert!(iter.next().is_none());

        let left_pa = left.paddr();
        let right_pa = right.paddr();
        assert_ne!(right_pa, left_pa + mm::PAGE_SIZE);

        let mut staged = [left, right].into_iter();
        let stack = KernelStack::try_new_with_guard_page(2, || Ok(staged.next().unwrap())).unwrap();

        let base = stack.kvirt_area.start();
        assert_eq!(stack.kvirt_area.size(), 6 * mm::PAGE_SIZE);
        assert_eq!(stack.end_vaddr(), base + 4 * mm::PAGE_SIZE);

        {
            // Disable local IRQs while querying kernel page-table nodes in
            // `VMALLOC_VADDR_RANGE` to match `KVirtArea`'s IRQ-safe lock order.
            let irq_guard = irq::disable_local();
            for guard_index in [0, 1, 4, 5] {
                assert!(
                    stack
                        .kvirt_area
                        .query(&irq_guard, base + guard_index * mm::PAGE_SIZE)
                        .is_none()
                );
            }

            let mut paddrs = Vec::with_capacity(2);
            for page_index in 0..2 {
                let addr = base + (2 + page_index) * mm::PAGE_SIZE;
                let MappedItemRef::Tracked(frame, prop) =
                    stack.kvirt_area.query(&irq_guard, addr).unwrap()
                else {
                    panic!("expected a tracked kernel stack frame");
                };
                assert_eq!(prop.flags & PageFlags::RWX, PageFlags::RW);
                assert_eq!(prop.cache, CachePolicy::Writeback);
                paddrs.push(frame.paddr());
            }
            assert_eq!(paddrs, [left_pa, right_pa]);
        }

        let default_stack = KernelStack::new_with_guard_page().unwrap();
        let default_pages = super::STACK_SIZE_IN_PAGES as usize;
        assert_eq!(
            default_stack.kvirt_area.size(),
            super::KERNEL_STACK_SIZE + 2 * super::GUARD_SIZE
        );
        assert_eq!(
            default_stack.end_vaddr(),
            default_stack.kvirt_area.start() + super::GUARD_SIZE + default_pages * mm::PAGE_SIZE
        );
    }
}
