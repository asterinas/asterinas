// SPDX-License-Identifier: MPL-2.0

#![feature(allocator_api)]
#![no_std]
#![deny(unsafe_code)]

mod allocator;
mod slab_cache;

pub use allocator::{type_from_layout, HeapAllocator};
