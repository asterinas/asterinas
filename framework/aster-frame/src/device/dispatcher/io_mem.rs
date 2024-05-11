// SPDX-License-Identifier: MPL-2.0

use alloc::{borrow::ToOwned, vec::Vec};
use core::ops::Range;

use align_ext::AlignExt;
use id_alloc::IdAlloc;
use log::{debug, info};

use crate::{
    boot::memory_region::MemoryRegion, device::io_mem::IoMem, sync::SpinLock, vm::PAGE_SIZE,
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
    unsafe fn add_range(&self, range: Range<usize>) {
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
    #[cfg(target_arch = "x86_64")]
    init_dispatcher(crate::boot::memory_regions().to_owned());
}

/// This function will initialize the allocatable MMIO area based on the x86-64 memory distribution map.
///
/// In x86-64, the available physical memory area is divided into two regions below 32 bits (Low memory)
/// and above (High memory). Where the area from the top of low memory to 0xffff_ffff, after the top of
/// high memory, is the available MMIO area.
///
#[cfg(target_arch = "x86_64")]
fn init_dispatcher(mut regions: Vec<MemoryRegion>) {
    use crate::boot::memory_region::MemoryRegionType;

    if regions.is_empty() {
        panic!("Memory Regions is empty.")
    }
    regions.sort_by_key(|a| a.base());
    regions.reverse();

    let mut is_tohm_found = false;
    const HIGH_MMIO_SIZE: usize = 0x100_0000;
    for region in regions {
        if !is_tohm_found && (region.base() + region.len()) > u32::MAX as usize {
            // Find the TOHM (Top of High Memory) and initialize High MMIO region.
            // Here, using 16MB (0x100_0000) as the size of High MMIO region.
            // TODO: Update the High MMIO region in runtime.
            if region.typ() == MemoryRegionType::Usable {
                is_tohm_found = true;
                // Align start address to 0x8_0000_0000
                let mmio_start_addr = (region.base() + region.len()).align_up(0x8_0000_0000);
                // SAFETY: The range is guaranteed not to access physical memory.
                unsafe {
                    IO_MEM_DISPATCHER
                        .add_range(mmio_start_addr..(mmio_start_addr + HIGH_MMIO_SIZE));
                }
            }
        } else if (region.base() + region.len()) < u32::MAX as usize {
            // Find the TOLM (Top of Low Memory) and initialize Low MMIO region (TOLM ~ 0xffff_ffff).
            if region.typ() == MemoryRegionType::Usable {
                // Align start address to 0x1000_0000
                let mmio_start_addr = (region.base() + region.len()).align_up(0x1000_0000);
                // SAFETY: The range is guaranteed not to access physical memory.
                unsafe {
                    IO_MEM_DISPATCHER.add_range(mmio_start_addr..0x1_0000_0000);
                }
                break;
            }
        }
    }
    if !is_tohm_found {
        let mmio_start_addr = 0x8_0000_0000;
        // SAFETY: The range is guaranteed not to access physical memory.
        unsafe {
            IO_MEM_DISPATCHER.add_range(mmio_start_addr..(mmio_start_addr + HIGH_MMIO_SIZE));
        }
    }
}
