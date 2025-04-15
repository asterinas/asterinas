// SPDX-License-Identifier: MPL-2.0

//! I/O port allocator.
use core::ops::Range;

use id_alloc::IdAlloc;
use log::debug;
use spin::Once;

use super::IoPort;
use crate::{
    io::RawIoPortRange,
    sync::{LocalIrqDisabled, SpinLock},
};

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

pub(super) static IO_PORT_ALLOCATOR: Once<IoPortAllocator> = Once::new();

/// Initializes the static `IO_PORT_ALLOCATOR` and removes the system device I/O port regions.
///
/// # Safety
///
/// User must ensure that:
///
/// 1. All the port I/O regions belonging to the system device are defined using the macros
///    `sensitive_io_port` and `reserve_io_port_range`.
///
/// 2. `MAX_IO_PORT` defined in `crate::arch::io` is guaranteed not to exceed the maximum
///    value specified by architecture.
pub(crate) unsafe fn init() {
    // SAFETY: `MAX_IO_PORT` is guaranteed not to exceed the maximum value specified by architecture.
    let mut allocator = IdAlloc::with_capacity(crate::arch::io::MAX_IO_PORT as usize);

    extern "C" {
        fn __sensitive_io_ports_start();
        fn __sensitive_io_ports_end();
    }
    let start = __sensitive_io_ports_start as usize;
    let end = __sensitive_io_ports_end as usize;
    assert!((end - start) % size_of::<RawIoPortRange>() == 0);

    // Iterate through the sensitive I/O port ranges and remove them from the allocator.
    let io_port_range_count = (end - start) / size_of::<RawIoPortRange>();
    for i in 0..io_port_range_count {
        let range_base_addr = __sensitive_io_ports_start as usize + i * size_of::<RawIoPortRange>();
        // SAFETY: The range is guaranteed to be valid as it is defined in the `.sensitive_io_ports` section.
        let port_range = unsafe { *(range_base_addr as *const RawIoPortRange) };

        assert!(port_range.begin < port_range.end);
        debug!("Removing sensitive I/O port range: {:#x?}", port_range);

        for i in port_range.begin..port_range.end {
            allocator.alloc_specific(i as usize);
        }
    }

    IO_PORT_ALLOCATOR.call_once(|| IoPortAllocator {
        allocator: SpinLock::new(allocator),
    });
}
