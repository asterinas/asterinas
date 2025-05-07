// SPDX-License-Identifier: MPL-2.0

//! Invalidation-related registers

use core::ptr::NonNull;

use volatile::{
    access::{ReadOnly, ReadWrite, WriteOnly},
    VolatileRef,
};

use super::ExtendedCapability;

#[derive(Debug)]
pub struct InvalidationRegisters {
    pub(super) queue_head: VolatileRef<'static, u64, ReadOnly>,
    pub(super) queue_tail: VolatileRef<'static, u64, ReadWrite>,
    pub(super) queue_addr: VolatileRef<'static, u64, ReadWrite>,

    pub(super) completion_status: VolatileRef<'static, u32, ReadWrite>,
    pub(super) _completion_event_control: VolatileRef<'static, u32, ReadWrite>,
    pub(super) _completion_event_data: VolatileRef<'static, u32, ReadWrite>,
    pub(super) _completion_event_addr: VolatileRef<'static, u32, ReadWrite>,
    pub(super) _completion_event_upper_addr: VolatileRef<'static, u32, ReadWrite>,

    pub(super) _queue_error_record: VolatileRef<'static, u64, ReadOnly>,

    pub(super) _invalidate_address: VolatileRef<'static, u64, WriteOnly>,
    pub(super) iotlb_invalidate: VolatileRef<'static, u64, ReadWrite>,
}

impl InvalidationRegisters {
    /// Creates an instance from the IOMMU base address.
    ///
    /// # Safety
    ///
    /// The caller must ensure that the base address is a valid IOMMU base address and that it has
    /// exclusive ownership of the IOMMU invalidation registers.
    pub(super) unsafe fn new(base: NonNull<u8>) -> Self {
        let offset = {
            // SAFETY: The safety is upheld by the caller.
            let extended_capability =
                unsafe { VolatileRef::new_read_only(base.add(0x10).cast::<u64>()) };
            let extend_cap = ExtendedCapability::new(extended_capability.as_ptr().read());
            extend_cap.iotlb_register_offset() as usize * 16
        };

        // FIXME: We now trust the hardware. We should instead find a way to check that `offset`
        // are reasonable values before proceeding.

        // SAFETY: The safety is upheld by the caller and the correctness of the capability value.
        unsafe {
            Self {
                queue_head: VolatileRef::new_read_only(base.add(0x80).cast::<u64>()),
                queue_tail: VolatileRef::new(base.add(0x88).cast::<u64>()),
                queue_addr: VolatileRef::new(base.add(0x90).cast::<u64>()),
                completion_status: VolatileRef::new(base.add(0x9C).cast::<u32>()),
                _completion_event_control: VolatileRef::new(base.add(0xA0).cast::<u32>()),
                _completion_event_data: VolatileRef::new(base.add(0xA4).cast::<u32>()),
                _completion_event_addr: VolatileRef::new(base.add(0xA8).cast::<u32>()),
                _completion_event_upper_addr: VolatileRef::new(base.add(0xAC).cast::<u32>()),
                _queue_error_record: VolatileRef::new_read_only(base.add(0xB0).cast::<u64>()),

                _invalidate_address: VolatileRef::new_restricted(
                    WriteOnly,
                    base.add(offset).cast::<u64>(),
                ),
                iotlb_invalidate: VolatileRef::new(base.add(offset).add(0x08).cast::<u64>()),
            }
        }
    }
}
