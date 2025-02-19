// SPDX-License-Identifier: MPL-2.0

#![feature(allocator_api)]
#![feature(generic_const_exprs)]
#![allow(incomplete_features)]
#![no_std]
#![deny(unsafe_code)]

mod allocator;
mod slab_cache;

pub use allocator::HeapAllocator;
