// SPDX-License-Identifier: MPL-2.0

pub struct InterruptEntryCache(pub u128);

impl InterruptEntryCache {
    pub fn global_invalidation() -> Self {
        Self(0x4)
    }
}
