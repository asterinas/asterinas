// SPDX-License-Identifier: MPL-2.0

use crate::{
    mm::{kspace::KERNEL_PAGE_TABLE, FrameAllocOptions, Paddr, PageFlags, Segment, PAGE_SIZE},
    prelude::*,
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

#[derive(Debug)]
pub struct KernelStack {
    segment: Segment,
    has_guard_page: bool,
}

impl KernelStack {
    /// Generates a kernel stack with a guard page.
    /// An additional page is allocated and be regarded as a guard page, which should not be accessed.  
    pub fn new_with_guard_page() -> Result<Self> {
        let stack_segment =
            FrameAllocOptions::new(STACK_SIZE_IN_PAGES as usize + 1).alloc_contiguous()?;
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
                .protect_flush_tlb(&vaddr_range, |p| p.flags -= PageFlags::RW)
                .unwrap();
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
                    .protect_flush_tlb(&vaddr_range, |p| p.flags |= PageFlags::RW)
                    .unwrap();
            }
        }
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
