// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::Ordering;

use crate::{
    arch::mm::tlb_flush_addr_range,
    cpu::{AtomicCpuSet, CpuSet, PinCurrentCpu},
    impl_frame_meta_for,
    mm::{
        kspace::kvirt_area::KVirtArea,
        page_prop::{CachePolicy, PageFlags, PageProperty, PrivilegedPageFlags},
        FrameAllocOptions, PAGE_SIZE,
    },
    prelude::*,
    trap::irq::DisabledLocalIrqGuard,
};

/// The kernel stack size of a task, specified in pages.
///
/// By default, we choose a rather large stack size.
/// OSTD users can choose a smaller size by specifying
/// the `OSTD_TASK_STACK_SIZE_IN_PAGES` environment variable
/// at build time.
pub static STACK_SIZE_IN_PAGES: u32 = parse_u32_or_default(
    option_env!("OSTD_TASK_STACK_SIZE_IN_PAGES"),
    DEFAULT_STACK_SIZE_IN_PAGES,
);

/// The default kernel stack size of a task, specified in pages.
pub const DEFAULT_STACK_SIZE_IN_PAGES: u32 = 128;

pub static KERNEL_STACK_SIZE: usize = STACK_SIZE_IN_PAGES as usize * PAGE_SIZE;

#[derive(Debug)]
#[expect(dead_code)]
pub struct KernelStack {
    kvirt_area: KVirtArea,
    tlb_coherent: AtomicCpuSet,
    end_vaddr: Vaddr,
    has_guard_page: bool,
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
    pub fn new_with_guard_page() -> Result<Self> {
        let pages = FrameAllocOptions::new()
            .zeroed(false)
            .alloc_segment_with(KERNEL_STACK_SIZE / PAGE_SIZE, |_| KernelStackMeta)?;
        let prop = PageProperty {
            has_map: true,
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
            has_guard_page: true,
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

    pub fn end_vaddr(&self) -> Vaddr {
        self.end_vaddr
    }
}

const fn parse_u32_or_default(size: Option<&str>, default: u32) -> u32 {
    match size {
        Some(value) => parse_u32(value),
        None => default,
    }
}

const fn parse_u32(input: &str) -> u32 {
    let mut output: u32 = 0;
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let digit = (bytes[i] - b'0') as u32;
        output = output * 10 + digit;
        i += 1;
    }
    output
}
