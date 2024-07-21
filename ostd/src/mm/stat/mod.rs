// SPDX-License-Identifier: MPL-2.0

//! APIs for memory statistics.

use crate::mm::page::allocator::PAGE_ALLOCATOR;

/// Total memory available for any usages in the system (in bytes).
///
/// It would be only a slightly less than total physical memory of the system
/// in most occasions. For example, bad memory, kernel statically-allocated
/// memory or firmware reserved memories do not count.
pub fn mem_total() -> usize {
    PAGE_ALLOCATOR.get().unwrap().lock().mem_total()
}

/// Current readily available memory (in bytes).
///
/// Such memory can be directly used for allocation without reclaiming.
pub fn mem_available() -> usize {
    PAGE_ALLOCATOR.get().unwrap().lock().mem_available()
}
