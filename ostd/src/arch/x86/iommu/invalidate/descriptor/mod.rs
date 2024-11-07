// SPDX-License-Identifier: MPL-2.0

pub struct InterruptEntryCache(pub u128);

impl InterruptEntryCache {
    const INVALIDATION_TYPE: u128 = 4;

    pub fn global_invalidation() -> Self {
        Self(Self::INVALIDATION_TYPE)
    }
}

pub struct InvalidationWait(pub u128);

impl InvalidationWait {
    const INVALIDATION_TYPE: u128 = 5;

    pub fn with_interrupt_flag() -> Self {
        Self(Self::INVALIDATION_TYPE | 0x10)
    }
}
