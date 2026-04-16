// SPDX-License-Identifier: MPL-2.0

#![feature(allocator_api)]
#![no_std]
#![deny(unsafe_code)]

extern crate alloc;

// Set this crate's log prefix for `ostd::log`.
macro_rules! __log_prefix {
    () => {
        "heap: "
    };
}

mod allocator;
mod cpu_local_allocator;
mod slab_cache;

pub use allocator::{HeapAllocator, type_from_layout};
pub use cpu_local_allocator::{CpuLocalBox, alloc_cpu_local};
