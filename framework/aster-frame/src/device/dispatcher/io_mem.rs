// SPDX-License-Identifier: MPL-2.0

use alloc::{borrow::ToOwned, vec::Vec};
use core::ops::Range;

use align_ext::AlignExt;
use id_alloc::IdAlloc;
use log::{debug, info};

use crate::{
    arch::dispatcher::init_io_mem_dispatcher, device::io_mem::IoMem, sync::SpinLock, vm::PAGE_SIZE
};

struct IoMemAllocator {
    /// Each ID indicates whether a memory I/O page (4KB) is allocated.
    ///
    /// TODO: Use a data structure that takes up less memory.
    allocator: IdAlloc,
    range: Range<usize>,
}

impl IoMemAllocator {
    fn new(range: Range<usize>) -> Self {
        let start_addr = range.start.align_up(PAGE_SIZE);
        let end_addr = range.end.align_down(PAGE_SIZE);
        let size = end_addr - start_addr;
        Self {
            allocator: IdAlloc::with_capacity(size / PAGE_SIZE),
            range: start_addr..end_addr,
        }
    }
}

pub struct IoMemDispatcher {
    /// Multiple allocators. For convenience, the ranges managed by allocator cannot be continuous.
    allocators: SpinLock<Vec<IoMemAllocator>>,
}

impl IoMemDispatcher {
    /// Get the `IoMem` based on the specified range. Return None if any region in 'range' cannot be allocated.
    ///
    /// The returned IoMem takes 4KB memory page as the smallest unit, if the range is not aligned
    /// to 4KB, user need to pay attention to add `offset`` when reading and writing.
    ///
    pub fn get(&self, range: Range<usize>) -> Option<IoMem> {
        let mut allocators = self.allocators.lock_irq_disabled();
        let mut result = None;
        for allocator in allocators.iter_mut() {
            if allocator.range.start >= range.end || allocator.range.end <= range.start {
                continue;
            }

            let start_addr = range.start.align_down(PAGE_SIZE);
            let end_addr = range.end.align_up(PAGE_SIZE);
            debug!("Allocating MMIO range:{:x?}..{:x?}", start_addr, end_addr);
            if allocator.range.end < end_addr || allocator.range.start > start_addr {
                return None;
            }

            let start_index = (start_addr - allocator.range.start) / PAGE_SIZE;
            let end_index = (end_addr - allocator.range.start) / PAGE_SIZE;
            if (start_index..end_index).any(|i| allocator.allocator.is_allocated(i)) {
                return None;
            }

            for index in start_index..end_index {
                allocator.allocator.alloc_specific(index);
            }

            // SAFETY: The created IoMem is guaranteed not to access physical memory or system device I/O
            unsafe { result = Some(IoMem::new(start_addr..end_addr)) }

            break;
        }
        result
    }

    /// Remove access to a specific memory IO range.  
    ///
    /// All drivers in the Framework must use this method to prevent peripheral drivers from accessing illegal memory IO range.
    pub(crate) fn remove(&self, range: Range<usize>) {
        let mut allocators = self.allocators.lock_irq_disabled();
        for allocator in allocators.iter_mut() {
            if allocator.range.start >= range.end || allocator.range.end <= range.start {
                continue;
            }

            let start_addr = range.start.align_down(PAGE_SIZE);
            let end_addr = range.end.align_up(PAGE_SIZE);
            info!("Removing MMIO range:{:x}..{:x}", start_addr, end_addr);

            let start_index = if start_addr > allocator.range.start {
                (start_addr - allocator.range.start) / PAGE_SIZE
            } else {
                0
            };
            let end_index = if end_addr < allocator.range.end {
                (end_addr - allocator.range.start) / PAGE_SIZE
            } else {
                (allocator.range.end - allocator.range.start) / PAGE_SIZE
            };

            // Remove I/O memory pages.
            for index in start_index..end_index {
                allocator.allocator.alloc_specific(index);
            }
        }
    }

    /// Add usable memory I/O region.
    ///
    /// # Safety
    ///
    /// User must ensure the range doesn't belong to physical memory.
    pub(crate) unsafe fn add_range(&self, range: Range<usize>) {
        let start_addr = range.start.align_up(PAGE_SIZE);
        let end_addr = range.end.align_down(PAGE_SIZE);
        info!("Adding MMIO range:{:x}...{:x}", start_addr, end_addr);
        self.allocators
            .lock_irq_disabled()
            .push(IoMemAllocator::new(start_addr..end_addr));
    }

    const fn new() -> Self {
        Self {
            allocators: SpinLock::new(Vec::new()),
        }
    }
}

pub static IO_MEM_DISPATCHER: IoMemDispatcher = IoMemDispatcher::new();

pub(crate) fn init() {
    // Initialize the allocator of the IoMemDispatcher based on the system memory info.
    init_io_mem_dispatcher(crate::boot::memory_regions().to_owned(), &IO_MEM_DISPATCHER);
}
