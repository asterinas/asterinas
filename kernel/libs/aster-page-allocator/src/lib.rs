// SPDX-License-Identifier: MPL-2.0

//! An implementation of the global page allocator for
//! [OSTD](https://crates.io/crates/ostd) based kernels.
//!
//! # Background
//!
//! `OSTD` has provided a page allocator interface, namely [`PageAlloc`] trait
//! and [`page_allocator_init_fn`] procedure macro, allowing users to plug in
//! their own page allocator wherever they want. You can refer to
//! `ostd/src/mm/page/allocator.rs` for detailed introduction.
//!
//! # Introduction
//!
//! This crate is the template of how to inject a page allocator into the OSTD.
//! Currently, the page allocator is implemented by the buddy system, as the
//! details listed in the file `buddy_allocator.rs`.
//!
//! This file is the entry point of the page allocator. It provides the
//! `init` function to initialize the global page allocator.

#![no_std]
#![deny(unsafe_code)]

extern crate alloc;
extern crate ostd;
pub(crate) mod buddy_allocator;

use alloc::boxed::Box;
use core::ops::Range;

use align_ext::AlignExt;
use buddy_allocator::BuddyFrameAllocator;
use log::info;
use ostd::{
    boot::memory_region::MemoryRegionType,
    mm::{page, page::allocator::PageAlloc, PAGE_SIZE},
};

pub fn init() -> Box<dyn PageAlloc> {
    let regions = ostd::boot::memory_regions();
    let mut allocator = Box::new(BuddyFrameAllocator::<32>::new());
    for region in regions.iter() {
        if region.typ() == MemoryRegionType::Usable {
            // Make the memory region page-aligned, and skip if it is too small.
            let start = region.base().align_up(PAGE_SIZE) / PAGE_SIZE;
            let region_end = region.base().checked_add(region.len()).unwrap();
            let end = region_end.align_down(PAGE_SIZE) / PAGE_SIZE;
            if end <= start {
                continue;
            }
            // Add global free pages to the frame allocator.
            allocator.add_free_pages(Range { start, end });
            info!(
                "Found usable region, start:{:x}, end:{:x}",
                region.base(),
                region.base() + region.len()
            );

            for frame in start..end {
                if page::Page::<page::meta::FrameMeta>::is_page_allocated(frame * PAGE_SIZE) {
                    allocator.alloc_specific(frame, frame + 1);
                }
            }
        }
    }
    info!(
        "Global page allocator is initialized, total memory: {}, allocated memory: {}",
        (allocator.total_mem()) / PAGE_SIZE,
        (allocator.total_mem() - allocator.free_mem()) / PAGE_SIZE
    );

    allocator
}
