// SPDX-License-Identifier: MPL-2.0

#![feature(allocator_api)]
#![no_std]
#![deny(unsafe_code)]

extern crate alloc;

mod allocator;
mod cpu_local_allocator;
mod slab_cache;

pub use allocator::{type_from_layout, HeapAllocator};
pub use cpu_local_allocator::{alloc_cpu_local, CpuLocalBox};
