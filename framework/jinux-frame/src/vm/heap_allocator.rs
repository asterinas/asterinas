use crate::{config::KERNEL_HEAP_SIZE, sync::SpinLock};
use buddy_system_allocator::Heap;
use core::{
    alloc::{GlobalAlloc, Layout},
    ptr::NonNull,
};

#[global_allocator]
static HEAP_ALLOCATOR: LockedHeap<32> = LockedHeap::new();

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

struct LockedHeap<const ORDER: usize>(SpinLock<Heap<ORDER>>);

impl<const ORDER: usize> LockedHeap<ORDER> {
    /// Creates an new heap
    pub const fn new() -> Self {
        LockedHeap(SpinLock::new(Heap::<ORDER>::new()))
    }

    /// Safety: The range [start, start + size) must be a valid memory region.
    pub unsafe fn init(&self, start: *const u8, size: usize) {
        self.0.lock().init(start as usize, size);
    }
}

unsafe impl<const ORDER: usize> GlobalAlloc for LockedHeap<ORDER> {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        self.0
            .lock()
            .alloc(layout)
            .map_or(0 as *mut u8, |allocation| allocation.as_ptr())
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        debug_assert!(ptr as usize != 0);
        self.0.lock().dealloc(NonNull::new_unchecked(ptr), layout)
    }
}
