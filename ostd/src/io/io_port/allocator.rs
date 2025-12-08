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
pub(super) struct IoPortAllocator {
    /// Each ID indicates whether a Port I/O (1B) is allocated.
    ///
    /// Instead of using `RangeAllocator` like `IoMemAllocator` does, it is more reasonable to use `IdAlloc`,
    /// as PIO space includes only a small region; for example, x86 module in OSTD allows just 65536 I/O ports.
    allocator: SpinLock<IdAlloc, LocalIrqDisabled>,
}

impl IoPortAllocator {
    /// Acquires an `IoPort`. Returns `None` if the PIO range is unavailable.
    ///
    /// `is_overlapping` indicates whether another `IoPort` can have a PIO range that overlaps with
    /// this one. If it is true, only the first port in the PIO range will be marked as occupied;
    /// otherwise, all ports in the PIO range will be marked as occupied.
    pub(super) fn acquire<T, A>(&self, port: u16, is_overlapping: bool) -> Option<IoPort<T, A>> {
        let range = if !is_overlapping {
            port..port.checked_add(size_of::<T>().try_into().ok()?)?
        } else {
            port..port.checked_add(1)?
        };
        debug!("Try to acquire PIO range: {:#x?}", range);

        let mut allocator = self.allocator.lock();
        if range.clone().any(|i| allocator.is_allocated(i as usize)) {
            return None;
        }

        for i in range.clone() {
            allocator.alloc_specific(i as usize);
        }

        // SAFETY: The created `IoPort` is guaranteed not to access system device I/O.
        unsafe { Some(IoPort::new_overlapping(port, is_overlapping)) }
    }

    /// Recycles an PIO range.
    ///
    /// # Safety
    ///
    /// The caller must have ownership of the PIO region through the `IoPortAllocator::acquire` interface.
    pub(super) unsafe fn recycle(&self, range: Range<u16>) {
        debug!("Recycling PIO range: {:#x?}", range);

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
pub(in crate::io) unsafe fn init() {
    // SAFETY: `MAX_IO_PORT` is guaranteed not to exceed the maximum value specified by architecture.
    let mut allocator = IdAlloc::with_capacity(crate::arch::io::MAX_IO_PORT as usize);

    unsafe extern "C" {
        fn __sensitive_io_ports_start();
        fn __sensitive_io_ports_end();
    }
    let start = __sensitive_io_ports_start as *const () as usize;
    let end = __sensitive_io_ports_end as *const () as usize;
    assert!((end - start) % size_of::<RawIoPortRange>() == 0);

    // Iterate through the sensitive I/O port ranges and remove them from the allocator.
    let io_port_range_count = (end - start) / size_of::<RawIoPortRange>();
    for i in 0..io_port_range_count {
        let range_base_addr =
            __sensitive_io_ports_start as *const () as usize + i * size_of::<RawIoPortRange>();
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

#[cfg(ktest)]
mod test {
    use crate::{arch::device::io_port::ReadWriteAccess, prelude::*};

    type IoPort = crate::io::IoPort<u32, ReadWriteAccess>;
    type ByteIoPort = crate::io::IoPort<u8, ReadWriteAccess>;

    #[ktest]
    fn illegal_region() {
        let io_port_a = IoPort::acquire(0xffff);
        assert!(io_port_a.is_err());

        type IllegalIoPort = crate::io::IoPort<[u8; 0x10008], ReadWriteAccess>;
        let io_port_b = IllegalIoPort::acquire(0);
        assert!(io_port_b.is_err());
    }

    #[ktest]
    fn conflict_region() {
        let io_port_a = IoPort::acquire(0x60);
        assert!(io_port_a.is_ok());

        // This allocation will fail because its range conflicts with `io_port_a`.
        let io_port_b = IoPort::acquire(0x62);
        assert!(io_port_b.is_err());

        drop(io_port_a);

        // After dropping `io_port_a`, conflicts no longer exist so the allocation will succeed.
        let io_port_b = IoPort::acquire(0x62);
        assert!(io_port_b.is_ok());
    }

    #[ktest]
    fn overlapping_region() {
        // Reference: <https://wiki.osdev.org/PCI#Configuration_Space_Access_Mechanism_#1>
        let pci_data = IoPort::acquire_overlapping(0xcf8);
        // Reference: <https://www.intel.com/Assets/PDF/datasheet/290562.pdf>
        let rst_ctrl = ByteIoPort::acquire(0xcf9);
        assert!(pci_data.is_ok());
        assert!(rst_ctrl.is_ok());

        let pci_data2 = IoPort::acquire_overlapping(0xcf8);
        let rst_ctrl2 = ByteIoPort::acquire(0xcf9);
        assert!(pci_data2.is_err());
        assert!(rst_ctrl2.is_err());

        drop(pci_data);
        drop(rst_ctrl);

        let rst_ctrl3 = ByteIoPort::acquire(0xcf9);
        assert!(rst_ctrl3.is_ok());

        let pci_data3 = IoPort::acquire_overlapping(0xcf8);
        assert!(pci_data3.is_ok());
    }
}
