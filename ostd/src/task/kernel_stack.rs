// SPDX-License-Identifier: MPL-2.0

use crate::{
    mm::{
        kspace::kva::Kva,
        page::{allocator, meta::KernelStackMeta},
        PAGE_SIZE,
    },
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

pub static KERNEL_STACK_SIZE: usize = STACK_SIZE_IN_PAGES as usize * PAGE_SIZE;

#[derive(Debug)]
pub struct KernelStack {
    kva: Kva,
    end_vaddr: Vaddr,
    has_guard_page: bool,
}

impl KernelStack {
    /// Generates a kernel stack with a guard page.
    /// An additional page is allocated and be regarded as a guard page, which should not be accessed.
    pub fn new_with_guard_page() -> Result<Self> {
        let mut new_kva = Kva::new(KERNEL_STACK_SIZE + 4 * PAGE_SIZE);
        let mapped_start = new_kva.range().start + 2 * PAGE_SIZE;
        let mapped_end = mapped_start + KERNEL_STACK_SIZE;
        let pages = allocator::alloc(KERNEL_STACK_SIZE, |_| KernelStackMeta::default()).unwrap();
        unsafe {
            new_kva.map_pages(mapped_start..mapped_end, pages);
        }
        Ok(Self {
            kva: new_kva,
            end_vaddr: mapped_end,
            has_guard_page: true,
        })
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
