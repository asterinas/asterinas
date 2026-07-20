// SPDX-License-Identifier: MPL-2.0

#![feature(allocator_api)]
#![no_std]
#![deny(unsafe_code)]

extern crate alloc;

use core::sync::atomic::{AtomicUsize, Ordering};

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

/// Total size (in bytes) of physical memory committed to the kernel heap.
pub(crate) static TOTAL_HEAP_ALLOCATED: AtomicUsize = AtomicUsize::new(0);

/// Loads the total size (in bytes) of memory committed to the kernel heap.
///
/// Uses `Relaxed` ordering because the counter is eventually consistent and
/// read infrequently (only via `/proc/meminfo`).
pub fn load_total_heap_size() -> usize {
    TOTAL_HEAP_ALLOCATED.load(Ordering::Relaxed)
}
