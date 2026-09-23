// SPDX-License-Identifier: MPL-2.0

pub(in crate::arch::iommu) struct InterruptEntryCache(pub u128);

impl InterruptEntryCache {
    const INVALIDATION_TYPE: u128 = 4;

    pub(in crate::arch::iommu) fn global_invalidation() -> Self {
        Self(Self::INVALIDATION_TYPE)
    }
}

pub(in crate::arch::iommu) struct InvalidationWait(pub u128);

impl InvalidationWait {
    const INVALIDATION_TYPE: u128 = 5;

    pub(in crate::arch::iommu) fn with_interrupt_flag() -> Self {
        Self(Self::INVALIDATION_TYPE | 0x10)
    }
}
