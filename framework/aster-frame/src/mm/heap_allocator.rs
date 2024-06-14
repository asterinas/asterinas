// SPDX-License-Identifier: MPL-2.0

use core::{
    alloc::{GlobalAlloc, Layout},
    ptr::NonNull,
};

use align_ext::AlignExt;
use buddy_system_allocator::Heap;
use log::debug;

use super::paddr_to_vaddr;
use crate::{
    mm::{page::allocator::PAGE_ALLOCATOR, PAGE_SIZE},
    prelude::*,
    sync::SpinLock,
    trap::disable_local,
    Error,
};

#[global_allocator]
static HEAP_ALLOCATOR: LockedHeapWithRescue<32> = LockedHeapWithRescue::new(rescue);

#[alloc_error_handler]
pub fn handle_alloc_error(layout: core::alloc::Layout) -> ! {
    panic!("Heap allocation error, layout = {:?}", layout);
}

const INIT_KERNEL_HEAP_SIZE: usize = PAGE_SIZE * 256;

static mut HEAP_SPACE: [u8; INIT_KERNEL_HEAP_SIZE] = [0; INIT_KERNEL_HEAP_SIZE];

pub fn init() {
    // SAFETY: The HEAP_SPACE is a static memory range, so it's always valid.
    unsafe {
        HEAP_ALLOCATOR.init(HEAP_SPACE.as_ptr(), INIT_KERNEL_HEAP_SIZE);
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

    /// SAFETY: The range [start, start + size) must be a valid memory region.
    pub unsafe fn init(&self, start: *const u8, size: usize) {
        self.heap.lock_irq_disabled().init(start as usize, size);
    }

    /// SAFETY: The range [start, start + size) must be a valid memory region.
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
        let mut page_allocator = PAGE_ALLOCATOR.get().unwrap().lock();
        if num_frames >= MIN_NUM_FRAMES {
            page_allocator.alloc(num_frames).ok_or(Error::NoMemory)?
        } else {
            match page_allocator.alloc(MIN_NUM_FRAMES) {
                None => page_allocator.alloc(num_frames).ok_or(Error::NoMemory)?,
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

    // SAFETY: the frame is allocated from FramAllocator and never be deallocated,
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
