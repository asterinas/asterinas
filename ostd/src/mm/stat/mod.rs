// SPDX-License-Identifier: MPL-2.0

//! APIs for memory statistics.

use crate::mm::frame::allocator::FRAME_ALLOCATOR;

/// Total memory available for any usages in the system (in bytes).
///
/// It would be only a slightly less than total physical memory of the system
/// in most occasions. For example, bad memory, kernel statically-allocated
/// memory or firmware reserved memories do not count.
pub fn mem_total() -> usize {
    FRAME_ALLOCATOR.get().unwrap().lock().mem_total()
}

/// Current readily available memory (in bytes).
///
/// Such memory can be directly used for allocation without reclaiming.
pub fn mem_available() -> usize {
    FRAME_ALLOCATOR.get().unwrap().lock().mem_available()
}

#[cfg(ktest)]
mod allocator_tests {
    use super::*;
    use crate::{
        mm::{FrameAllocOptions, PAGE_SIZE},
        prelude::*,
    };

    #[ktest]
    fn allocator_counting() {
        let mem_total = mem_total();
        assert_ne!(mem_total, 0);
        let initial_available = mem_available();
        let frame = FrameAllocOptions::new().alloc_frame_with(()).unwrap();
        let after_alloc = mem_available();
        assert_eq!(initial_available - after_alloc, PAGE_SIZE);
        drop(frame);
        let after_free = mem_available();
        assert_eq!(after_free, initial_available);
    }
}
