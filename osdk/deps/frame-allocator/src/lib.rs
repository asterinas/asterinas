// SPDX-License-Identifier: MPL-2.0

#![feature(generic_const_exprs)]
#![allow(incomplete_features)]
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

use core::{
    alloc::Layout,
    sync::atomic::{AtomicIsize, Ordering},
};

use ostd::{
    cpu_local,
    mm::{frame::GlobalFrameAllocator, Paddr},
    trap,
};

pub(crate) mod cache;
pub(crate) mod chunk;
pub(crate) mod pools;
pub(crate) mod set;

cpu_local! {
    static TOTAL_FREE_SIZE: AtomicIsize = AtomicIsize::new(0);
}

/// Loads the total size (in bytes) of free memory in the allocator.
pub fn load_total_free_size() -> usize {
    let mut total: isize = 0;
    for cpu in ostd::cpu::all_cpus() {
        total += TOTAL_FREE_SIZE.get_on_cpu(cpu).load(Ordering::Relaxed);
    }
    total as usize
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
            TOTAL_FREE_SIZE
                .get_with(&guard)
                .fetch_sub(layout.size() as isize, Ordering::Relaxed);
        }
        res
    }

    fn dealloc(&self, addr: Paddr, size: usize) {
        let guard = trap::disable_local();
        TOTAL_FREE_SIZE
            .get_with(&guard)
            .fetch_add(size as isize, Ordering::Relaxed);
        cache::dealloc(&guard, addr, size);
    }
}
