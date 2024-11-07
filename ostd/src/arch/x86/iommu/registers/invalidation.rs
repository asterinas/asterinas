// SPDX-License-Identifier: MPL-2.0

//! Invalidation-related registers

use volatile::{
    access::{ReadOnly, ReadWrite, WriteOnly},
    Volatile,
};

use super::ExtendedCapability;
use crate::prelude::Vaddr;

#[derive(Debug)]
pub struct InvalidationRegisters {
    pub(super) queue_head: Volatile<&'static u64, ReadOnly>,
    pub(super) queue_tail: Volatile<&'static mut u64, ReadWrite>,
    pub(super) queue_addr: Volatile<&'static mut u64, ReadWrite>,

    pub(super) completion_status: Volatile<&'static mut u32, ReadWrite>,
    pub(super) _completion_event_control: Volatile<&'static mut u32, ReadWrite>,
    pub(super) _completion_event_data: Volatile<&'static mut u32, ReadWrite>,
    pub(super) _completion_event_addr: Volatile<&'static mut u32, ReadWrite>,
    pub(super) _completion_event_upper_addr: Volatile<&'static mut u32, ReadWrite>,

    pub(super) _queue_error_record: Volatile<&'static mut u64, ReadOnly>,

    pub(super) _invalidate_address: Volatile<&'static mut u64, WriteOnly>,
    pub(super) _iotlb_invalidate: Volatile<&'static mut u64, ReadWrite>,
}

impl InvalidationRegisters {
    /// Creates an instance from IOMMU base address.
    ///
    /// # Safety
    ///
    /// User must ensure the address is valid.
    pub(super) unsafe fn new(base_vaddr: Vaddr) -> Self {
        let extended_capability: Volatile<&u64, ReadOnly> =
            Volatile::new_read_only(&*((base_vaddr + 0x10) as *const u64));
        let extend_cap = ExtendedCapability::new(extended_capability.read());
        let offset = extend_cap.iotlb_register_offset() as usize * 16;

        let invalidate_address =
            Volatile::new_write_only(&mut *((base_vaddr + offset) as *mut u64));
        let iotlb_invalidate = Volatile::new(&mut *((base_vaddr + offset + 0x8) as *mut u64));

        Self {
            queue_head: Volatile::new_read_only(&*((base_vaddr + 0x80) as *mut u64)),
            queue_tail: Volatile::new(&mut *((base_vaddr + 0x88) as *mut u64)),
            queue_addr: Volatile::new(&mut *((base_vaddr + 0x90) as *mut u64)),
            completion_status: Volatile::new(&mut *((base_vaddr + 0x9C) as *mut u32)),
            _completion_event_control: Volatile::new(&mut *((base_vaddr + 0xA0) as *mut u32)),
            _completion_event_data: Volatile::new(&mut *((base_vaddr + 0xA4) as *mut u32)),
            _completion_event_addr: Volatile::new(&mut *((base_vaddr + 0xA8) as *mut u32)),
            _completion_event_upper_addr: Volatile::new(&mut *((base_vaddr + 0xAC) as *mut u32)),
            _queue_error_record: Volatile::new_read_only(&mut *((base_vaddr + 0xB0) as *mut u64)),
            _invalidate_address: invalidate_address,
            _iotlb_invalidate: iotlb_invalidate,
        }
    }
}
