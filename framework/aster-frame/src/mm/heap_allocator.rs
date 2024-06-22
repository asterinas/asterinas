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
    mm::{page::allocator::FRAME_ALLOCATOR, PAGE_SIZE},
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
mod heap_profile {
    use alloc::{alloc::Layout, collections::BTreeMap};

    use crate::sync::SpinLock;

    #[derive(Debug)]
    pub struct AllocRecord {
        pub layout: Layout,
        pub stack: [usize; 20],
    }

    impl AllocRecord {
        #[inline(always)]
        pub fn new(layout: Layout) -> Self {
            use core::ffi::c_void;

            use unwinding::abi::{
                UnwindContext, UnwindReasonCode, _Unwind_Backtrace, _Unwind_GetIP,
            };

            struct StackData {
                stack: [usize; 20],
                stack_top: usize,
            }

            extern "C" fn callback(
                unwind_ctx: &UnwindContext<'_>,
                arg: *mut c_void,
            ) -> UnwindReasonCode {
                let data = unsafe { &mut *(arg as *mut StackData) };
                let pc = _Unwind_GetIP(unwind_ctx);
                if data.stack_top < data.stack.len() {
                    data.stack[data.stack_top] = pc;
                    data.stack_top += 1;
                }
                UnwindReasonCode::NO_REASON
            }

            let mut data = StackData {
                stack: [0; 20],
                stack_top: 0,
            };
            _Unwind_Backtrace(callback, &mut data as *mut _ as _);
            let StackData { stack, stack_top: _ } = data;

            Self { layout, stack }
        }
    }

    static PROFILE_DATA: SpinLock<Option<BTreeMap<usize, AllocRecord>>> = SpinLock::new(None);

    /// Start heap profiling.
    pub fn start_heap_profile() {
        crate::early_println!("[kern] start heap profile");
        let old = PROFILE_DATA.lock().replace(BTreeMap::new());
        assert!(old.is_none());
    }

    /// Stop heap profiling and return the result.
    pub fn stop_heap_profile() -> BTreeMap<usize, AllocRecord> {
        crate::early_println!("[kern] stop heap profile");
        let result = PROFILE_DATA.lock().take().unwrap();
        result
    }

    #[inline(always)]
    pub(super) fn debug_profile(ptr: usize, layout: Layout) {
        if let Some(mut guard) = PROFILE_DATA.try_lock() {
            if let Some(profile_data) = guard.as_mut() {
                let record = AllocRecord::new(layout);
                profile_data.insert(ptr, record);
            }
        }
    }

    #[inline(always)]
    pub(super) fn debug_remove_profile(ptr: usize) {
        if let Some(mut guard) = PROFILE_DATA.try_lock() {
            if let Some(profile_data) = guard.as_mut() {
                let _ = profile_data.remove(&ptr);
            }
        }
    }
}
pub use heap_profile::{start_heap_profile, stop_heap_profile};

unsafe impl<const ORDER: usize> GlobalAlloc for LockedHeapWithRescue<ORDER> {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _guard = disable_local();

        let alloc_result = {
            let mut lock_guard = self.heap.lock();
            if let Ok(allocation) = lock_guard.alloc(layout) {
                Some(allocation.as_ptr())
            } else {
                None
            }
        };

        if let Some(ptr) = alloc_result {
            heap_profile::debug_profile(ptr as usize, layout);
            return ptr;
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
        heap_profile::debug_remove_profile(ptr as usize);
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
