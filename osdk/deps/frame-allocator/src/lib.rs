// SPDX-License-Identifier: MPL-2.0

#![no_std]
#![deny(unsafe_code)]

//! An implementation of the global physical memory frame allocator for
//! [OSTD](https://crates.io/crates/ostd) based kernels.
//!
//! # Background
//!
//! `OSTD` has provided a page allocator interface, namely [`GlobalFrameAllocator`]
//! and [`global_frame_allocator`] procedure macro, allowing users to plug in
//! their own frame allocator into the kernel safely. You can refer to the
//! [`ostd::mm::frame::allocator`] module for detailed introduction.
//!
//! # Introduction
//!
//! This crate is an implementation of a scalable and efficient global frame
//! allocator based on the buddy system. It is by default shipped with OSDK
//! for users that don't have special requirements on the frame allocator.
//!
//! [`GlobalFrameAllocator`]: ostd::mm::GlobalFrameAllocator
//! [`global_frame_allocator`]: ostd::global_frame_allocator

// The heap allocator usually depends on frame allocation. If we depend on heap
// allocation then there will be a cyclic dependency. We only use the heap in
// unit tests.
#[cfg(ktest)]
extern crate alloc;

use core::alloc::Layout;

use ostd::{
    mm::{frame::GlobalFrameAllocator, Paddr},
    trap,
};

mod cache;
mod chunk;
mod per_cpu_counter;
mod pools;
mod set;

#[cfg(ktest)]
mod test;

/// Loads the total size (in bytes) of free memory in the allocator.
pub fn load_total_free_size() -> usize {
    per_cpu_counter::read_total_free_size()
}

/// The global frame allocator provided by OSDK.
///
/// It is a singleton that provides frame allocation for the kernel. If
/// multiple instances of this struct are created, all the member functions
/// will eventually access the same allocator.
pub struct FrameAllocator;

impl GlobalFrameAllocator for FrameAllocator {
    fn alloc(&self, layout: Layout) -> Option<Paddr> {
        let guard = trap::disable_local();
        let res = cache::alloc(&guard, layout);
        if res.is_some() {
            per_cpu_counter::sub_free_size(&guard, layout.size());
        }
        res
    }

    fn dealloc(&self, addr: Paddr, size: usize) {
        let guard = trap::disable_local();
        per_cpu_counter::add_free_size(&guard, size);
        cache::dealloc(&guard, addr, size);
    }

    fn add_free_memory(&self, addr: Paddr, size: usize) {
        let guard = trap::disable_local();
        per_cpu_counter::add_free_size(&guard, size);
        pools::add_free_memory(&guard, addr, size);
    }
}
