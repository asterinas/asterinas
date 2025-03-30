// SPDX-License-Identifier: MPL-2.0

//! I/O port allocator.
use core::ops::Range;

use id_alloc::IdAlloc;
use log::{debug, info};
use spin::Once;

use super::IoPort;
use crate::sync::{LocalIrqDisabled, SpinLock};

/// I/O port allocator that allocates port I/O access to device drivers.
pub struct IoPortAllocator {
    /// Each ID indicates whether a Port I/O (1B) is allocated.
    ///
    /// Instead of using `RangeAllocator` like `IoMemAllocator` does, it is more reasonable to use `IdAlloc`,
    /// as PIO space includes only a small region; for example, x86 module in OSTD allows just 65536 I/O ports.
    allocator: SpinLock<IdAlloc, LocalIrqDisabled>,
}

impl IoPortAllocator {
    /// Acquires the `IoPort`. Return None if any region in `port` cannot be allocated.
    pub fn acquire<T, A>(&self, port: u16) -> Option<IoPort<T, A>> {
        let mut allocator = self.allocator.lock();
        let mut range = port..(port + size_of::<T>() as u16);
        if range.any(|i| allocator.is_allocated(i as usize)) {
            return None;
        }

        for i in range.clone() {
            allocator.alloc_specific(i as usize);
        }

        // SAFETY: The created IoPort is guaranteed not to access system device I/O
        unsafe { Some(IoPort::new(port)) }
    }

    /// Recycles an PIO range.
    ///
    /// # Safety
    ///
    /// The caller must have ownership of the PIO region through the `IoPortAllocator::acquire` interface.
    pub(in crate::io) unsafe fn recycle(&self, range: Range<u16>) {
        debug!("Recycling MMIO range: {:#x?}", range);

        self.allocator
            .lock()
            .free_consecutive(range.start as usize..range.end as usize);
    }
}

/// Builder for `IoPortAllocator`.
///
/// The builder must contains the port I/O regions according to architecture specification. Also, OSTD
/// must exclude the port I/O regions of the system device before building the `IoPortAllocator`.
pub(crate) struct IoPortAllocatorBuilder {
    allocator: IdAlloc,
}

impl IoPortAllocatorBuilder {
    /// Initializes port I/O region for devices.
    ///
    /// # Safety
    ///
    /// User must ensure `max_port` doesn't exceed the maximum value specified by architecture.
    pub(crate) unsafe fn new(max_port: u16) -> Self {
        info!(
            "Creating new I/O port allocator builder, max_port: {:#x?}",
            max_port
        );

        Self {
            allocator: IdAlloc::with_capacity(max_port as usize),
        }
    }

    /// Removes access to a specific port I/O range.  
    ///
    /// All drivers in OSTD must use this method to prevent peripheral drivers from accessing illegal port IO range.
    pub(crate) fn remove(&mut self, range: Range<u16>) {
        info!("Removing PIO range: {:#x?}", range);

        for i in range {
            self.allocator.alloc_specific(i as usize);
        }
    }
}

pub(super) static IO_PORT_ALLOCATOR: Once<IoPortAllocator> = Once::new();

/// Initializes the static `IO_PORT_ALLOCATOR` based on builder.
///
/// # Safety
///
/// User must ensure all the port I/O regions that belong to the system device have been removed by calling the
/// `remove` function.
pub(crate) unsafe fn init(io_port_builder: IoPortAllocatorBuilder) {
    IO_PORT_ALLOCATOR.call_once(|| IoPortAllocator {
        allocator: SpinLock::new(io_port_builder.allocator),
    });
}
