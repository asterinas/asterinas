// SPDX-License-Identifier: MPL-2.0

mod slab_allocator;

use core::alloc::{GlobalAlloc, Layout};

use align_ext::AlignExt;
use log::debug;
use slab_allocator::Heap;
use spin::Once;

use super::paddr_to_vaddr;
use crate::{
    mm::{page::allocator::PAGE_ALLOCATOR, PAGE_SIZE},
    prelude::*,
    sync::SpinLock,
    trap::disable_local,
    Error,
};

#[global_allocator]
static HEAP_ALLOCATOR: LockedHeapWithRescue = LockedHeapWithRescue::new();

#[alloc_error_handler]
pub fn handle_alloc_error(layout: core::alloc::Layout) -> ! {
    panic!("Heap allocation error, layout = {:?}", layout);
}

const INIT_KERNEL_HEAP_SIZE: usize = PAGE_SIZE * 256;

#[repr(align(4096))]
struct InitHeapSpace([u8; INIT_KERNEL_HEAP_SIZE]);

/// Initialize the heap allocator.
///
/// # Safety
///
/// This function should be called only once.
pub unsafe fn init() {
    static mut HEAP_SPACE: InitHeapSpace = InitHeapSpace([0; INIT_KERNEL_HEAP_SIZE]);
    // SAFETY: The HEAP_SPACE is a static memory range, so it's always valid.
    unsafe {
        #[allow(static_mut_refs)]
        HEAP_ALLOCATOR.init(HEAP_SPACE.0.as_ptr(), INIT_KERNEL_HEAP_SIZE);
    }
}

struct LockedHeapWithRescue {
    heap: Once<SpinLock<Heap>>,
}

impl LockedHeapWithRescue {
    /// Creates an new heap
    pub const fn new() -> Self {
        Self { heap: Once::new() }
    }

    /// SAFETY: The range [start, start + size) must be a valid memory region.
    pub unsafe fn init(&self, start: *const u8, size: usize) {
        self.heap
            .call_once(|| SpinLock::new(Heap::new(start as usize, size)));
    }

    /// SAFETY: The range [start, start + size) must be a valid memory region.
    unsafe fn add_to_heap(&self, start: usize, size: usize) {
        self.heap
            .get()
            .unwrap()
            .disable_irq()
            .lock()
            .add_memory(start, size);
    }

    fn rescue_if_low_memory(&self, remain_bytes: usize, layout: Layout) {
        if remain_bytes <= PAGE_SIZE * 4 {
            debug!(
                "Low memory in heap allocator, try to call rescue. Remaining bytes: {:x?}",
                remain_bytes
            );
            // We don't care if the rescue returns ok or not since we can still do heap allocation.
            let _ = self.rescue(&layout);
        }
    }

    fn rescue(&self, layout: &Layout) -> Result<()> {
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

        // SAFETY: the frame is allocated from FrameAllocator and never be deallocated,
        // so the addr is always valid.
        unsafe {
            debug!(
                "add frames to heap: addr = 0x{:x}, size = 0x{:x}",
                vaddr,
                PAGE_SIZE * num_frames
            );
            self.add_to_heap(vaddr, PAGE_SIZE * num_frames);
        }

        Ok(())
    }
}

unsafe impl GlobalAlloc for LockedHeapWithRescue {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _guard = disable_local();

        let res = self.heap.get().unwrap().lock().allocate(layout);
        if let Ok((allocation, remain_bytes)) = res {
            self.rescue_if_low_memory(remain_bytes, layout);
            return allocation;
        }

        if self.rescue(&layout).is_err() {
            return core::ptr::null_mut::<u8>();
        }

        let res = self.heap.get().unwrap().lock().allocate(layout);
        if let Ok((allocation, remain_bytes)) = res {
            self.rescue_if_low_memory(remain_bytes, layout);
            allocation
        } else {
            core::ptr::null_mut::<u8>()
        }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        debug_assert!(ptr as usize != 0);
        self.heap
            .get()
            .unwrap()
            .disable_irq()
            .lock()
            .deallocate(ptr, layout)
    }
}
