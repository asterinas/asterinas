// SPDX-License-Identifier: MPL-2.0

use crate::{
    arch::mm::tlb_flush_addr_range,
    mm::{kspace::KERNEL_PAGE_TABLE, FrameAllocOptions, Paddr, PageFlags, Segment, PAGE_SIZE},
    prelude::*,
};

pub const KERNEL_STACK_SIZE: usize = PAGE_SIZE * 64;

pub struct KernelStack {
    segment: Segment,
    has_guard_page: bool,
}

impl KernelStack {
    pub fn new() -> Result<Self> {
        Ok(Self {
            segment: FrameAllocOptions::new(KERNEL_STACK_SIZE / PAGE_SIZE).alloc_contiguous()?,
            has_guard_page: false,
        })
    }

    /// Generates a kernel stack with a guard page.
    /// An additional page is allocated and be regarded as a guard page, which should not be accessed.  
    pub fn new_with_guard_page() -> Result<Self> {
        let stack_segment =
            FrameAllocOptions::new(KERNEL_STACK_SIZE / PAGE_SIZE + 1).alloc_contiguous()?;
        // FIXME: modifying the the linear mapping is bad.
        let page_table = KERNEL_PAGE_TABLE.get().unwrap();
        let guard_page_vaddr = {
            let guard_page_paddr = stack_segment.start_paddr();
            crate::mm::paddr_to_vaddr(guard_page_paddr)
        };
        // SAFETY: the segment allocated is not used by others so we can protect it.
        unsafe {
            let vaddr_range = guard_page_vaddr..guard_page_vaddr + PAGE_SIZE;
            page_table
                .protect(&vaddr_range, |p| p.flags -= PageFlags::RW)
                .unwrap();
            tlb_flush_addr_range(&vaddr_range);
        }
        Ok(Self {
            segment: stack_segment,
            has_guard_page: true,
        })
    }

    pub fn end_paddr(&self) -> Paddr {
        self.segment.end_paddr()
    }
}

impl Drop for KernelStack {
    fn drop(&mut self) {
        if self.has_guard_page {
            // FIXME: modifying the the linear mapping is bad.
            let page_table = KERNEL_PAGE_TABLE.get().unwrap();
            let guard_page_vaddr = {
                let guard_page_paddr = self.segment.start_paddr();
                crate::mm::paddr_to_vaddr(guard_page_paddr)
            };
            // SAFETY: the segment allocated is not used by others so we can protect it.
            unsafe {
                let vaddr_range = guard_page_vaddr..guard_page_vaddr + PAGE_SIZE;
                page_table
                    .protect(&vaddr_range, |p| p.flags |= PageFlags::RW)
                    .unwrap();
                tlb_flush_addr_range(&vaddr_range);
            }
        }
    }
}
