// SPDX-License-Identifier: MPL-2.0

use crate::config::{KERNEL_HEAP_SIZE, PAGE_SIZE};
use crate::prelude::*;
use crate::sync::SpinLock;
use crate::trap::disable_local;
use crate::vm::frame_allocator::FRAME_ALLOCATOR;
use crate::Error;
use align_ext::AlignExt;
use buddy_system_allocator::Heap;
use core::{
    alloc::{GlobalAlloc, Layout},
    ptr::NonNull,
};
use log::debug;

use super::paddr_to_vaddr;

#[global_allocator]
static HEAP_ALLOCATOR: LockedHeapWithRescue<32> = LockedHeapWithRescue::new(rescue);

#[alloc_error_handler]
pub fn handle_alloc_error(layout: core::alloc::Layout) -> ! {
    panic!("Heap allocation error, layout = {:?}", layout);
}

static mut HEAP_SPACE: [u8; KERNEL_HEAP_SIZE] = [0; KERNEL_HEAP_SIZE];

pub fn init() {
    // Safety: The HEAP_SPACE is a static memory range, so it's always valid.
    unsafe {
        HEAP_ALLOCATOR.init(HEAP_SPACE.as_ptr(), KERNEL_HEAP_SIZE);
    }
}

struct LockedHeapWithRescue<const ORDER: usize> {
    heap: SpinLock<Heap<ORDER>>,
    rescue: fn(&Self, &Layout) -> Result<()>,
}

impl<const ORDER: usize> LockedHeapWithRescue<ORDER> {
    /// Creates an new heap
    pub const fn new(rescue: fn(&Self, &Layout) -> Result<()>) -> Self {
        Self {
            heap: SpinLock::new(Heap::<ORDER>::new()),
            rescue,
        }
    }

    /// Safety: The range [start, start + size) must be a valid memory region.
    pub unsafe fn init(&self, start: *const u8, size: usize) {
        self.heap.lock_irq_disabled().init(start as usize, size);
    }

    /// Safety: The range [start, start + size) must be a valid memory region.
    unsafe fn add_to_heap(&self, start: usize, size: usize) {
        self.heap
            .lock_irq_disabled()
            .add_to_heap(start, start + size)
    }
}

unsafe impl<const ORDER: usize> GlobalAlloc for LockedHeapWithRescue<ORDER> {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _guard = disable_local();

        if let Ok(allocation) = self.heap.lock().alloc(layout) {
            return allocation.as_ptr();
        }

        // Avoid locking self.heap when calling rescue.
        if (self.rescue)(self, &layout).is_err() {
            return core::ptr::null_mut::<u8>();
        }

        self.heap
            .lock()
            .alloc(layout)
            .map_or(core::ptr::null_mut::<u8>(), |allocation| {
                allocation.as_ptr()
            })
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        debug_assert!(ptr as usize != 0);
        self.heap
            .lock_irq_disabled()
            .dealloc(NonNull::new_unchecked(ptr), layout)
    }
}

fn rescue<const ORDER: usize>(heap: &LockedHeapWithRescue<ORDER>, layout: &Layout) -> Result<()> {
    const MIN_NUM_FRAMES: usize = 0x4000000 / PAGE_SIZE; // 64MB

    debug!("enlarge heap, layout = {:?}", layout);
    let mut num_frames = {
        let align = PAGE_SIZE.max(layout.align());
        debug_assert!(align % PAGE_SIZE == 0);
        let size = layout.size().align_up(align);
        size / PAGE_SIZE
    };

    let allocation_start = {
        let mut frame_allocator = FRAME_ALLOCATOR.get().unwrap().lock();
        if num_frames >= MIN_NUM_FRAMES {
            frame_allocator.alloc(num_frames).ok_or(Error::NoMemory)?
        } else {
            match frame_allocator.alloc(MIN_NUM_FRAMES) {
                None => frame_allocator.alloc(num_frames).ok_or(Error::NoMemory)?,
                Some(start) => {
                    num_frames = MIN_NUM_FRAMES;
                    start
                }
            }
        }
    };
    // FIXME: the alloc function internally allocates heap memory(inside FrameAllocator).
    // So if the heap is nearly run out, allocating frame will fail too.
    let vaddr = paddr_to_vaddr(allocation_start * PAGE_SIZE);

    // Safety: the frame is allocated from FramAllocator and never be deallocated,
    // so the addr is always valid.
    unsafe {
        debug!(
            "add frames to heap: addr = 0x{:x}, size = 0x{:x}",
            vaddr,
            PAGE_SIZE * num_frames
        );
        heap.add_to_heap(vaddr, PAGE_SIZE * num_frames);
    }

    Ok(())
}
