// SPDX-License-Identifier: MPL-2.0

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
static HEAP_ALLOCATOR: LockedHeapWithRescue = LockedHeapWithRescue::new(rescue);

#[alloc_error_handler]
pub fn handle_alloc_error(layout: core::alloc::Layout) -> ! {
    panic!("Heap allocation error, layout = {:?}", layout);
}

const INIT_KERNEL_HEAP_SIZE: usize = PAGE_SIZE * 256;

#[repr(align(4096))]
struct InitHeapSpace([u8; INIT_KERNEL_HEAP_SIZE]);

static mut HEAP_SPACE: InitHeapSpace = InitHeapSpace([0; INIT_KERNEL_HEAP_SIZE]);

pub fn init() {
    // SAFETY: The HEAP_SPACE is a static memory range, so it's always valid.
    unsafe {
        HEAP_ALLOCATOR.init(HEAP_SPACE.0.as_ptr(), INIT_KERNEL_HEAP_SIZE);
    }
}

struct LockedHeapWithRescue {
    heap: Once<SpinLock<Heap>>,
    rescue: fn(&Self, &Layout) -> Result<()>,
}

impl LockedHeapWithRescue {
    /// Creates an new heap
    pub const fn new(rescue: fn(&Self, &Layout) -> Result<()>) -> Self {
        Self {
            heap: Once::new(),
            rescue,
        }
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
}

unsafe impl GlobalAlloc for LockedHeapWithRescue {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _guard = disable_local();

        if let Ok(allocation) = self.heap.get().unwrap().lock().allocate(layout) {
            return allocation as *mut u8;
        }

        // Avoid locking self.heap when calling rescue.
        if (self.rescue)(self, &layout).is_err() {
            return core::ptr::null_mut::<u8>();
        }

        let res = self
            .heap
            .get()
            .unwrap()
            .lock()
            .allocate(layout)
            .map_or(core::ptr::null_mut::<u8>(), |allocation| {
                allocation as *mut u8
            });
        res
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        debug_assert!(ptr as usize != 0);
        self.heap
            .get()
            .unwrap()
            .disable_irq()
            .lock()
            .deallocate(ptr as usize, layout)
    }
}

fn rescue(heap: &LockedHeapWithRescue, layout: &Layout) -> Result<()> {
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
        heap.add_to_heap(vaddr, PAGE_SIZE * num_frames);
    }

    Ok(())
}
