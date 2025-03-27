// SPDX-License-Identifier: MPL-2.0

//! CPU ID.
//!
//! Not to be confused with the x86 `cpuid` instruction. This module provides
//! the identifier of a CPU [`CpuId`] in the system.

use core::sync::atomic::{AtomicU32, Ordering};

use super::num_cpus;

/// The ID of a CPU in the system.
///
/// If converting from/to an integer, the integer must start from 0 and be less
/// than the number of CPUs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CpuId(u32);

impl CpuId {
    /// Returns the CPU ID of the bootstrap processor (BSP).
    pub const fn bsp() -> Self {
        CpuId(0)
    }

    /// Converts the CPU ID to an `usize`.
    pub const fn as_usize(self) -> usize {
        self.0 as usize
    }

    /// Converts an `usize` to a `CpuId`.
    ///
    /// # Safety
    ///
    /// The given value must be less than the number of CPUs.
    pub(super) unsafe fn from_usize_unchecked(value: usize) -> Self {
        debug_assert!(value < num_cpus());
        CpuId(value as u32)
    }
}

impl TryFrom<usize> for CpuId {
    type Error = &'static str;

    fn try_from(value: usize) -> Result<Self, Self::Error> {
        if value < num_cpus() {
            Ok(CpuId(value as u32))
        } else {
            Err("The given CPU ID is out of range")
        }
    }
}

/// Returns an iterator over all CPUs.
pub fn all_cpus() -> impl Iterator<Item = CpuId> {
    (0..num_cpus()).map(|id| CpuId(id as u32))
}

/// An atomic version of `Option<CpuId>`.
#[derive(Debug)]
pub struct AtomicOptionCpuId {
    inner: AtomicU32,
}

impl AtomicOptionCpuId {
    const NONE_VALUE: u32 = u32::MAX;

    /// Creates a new `AtomicOptionCpuId`.
    pub const fn new(id: Option<CpuId>) -> Self {
        AtomicOptionCpuId {
            inner: AtomicU32::new(Self::val_from_id(id)),
        }
    }

    /// Loads the value with the given ordering.
    pub fn load(&self, ordering: Ordering) -> Option<CpuId> {
        let value = self.inner.load(ordering);
        Self::id_from_val(value)
    }

    /// Stores the value with the given ordering.
    pub fn store(&self, value: Option<CpuId>, ordering: Ordering) {
        self.inner.store(Self::val_from_id(value), ordering);
    }

    /// Swaps the value with the given ordering.
    pub fn swap(&self, value: Option<CpuId>, ordering: Ordering) -> Option<CpuId> {
        let value = self.inner.swap(Self::val_from_id(value), ordering);
        Self::id_from_val(value)
    }

    /// Does an atomic compare-exchange over the value with the given ordering.
    ///
    /// The memory ordering and specific behavior are the same as
    /// [`AtomicU32::compare_exchange`].
    pub fn compare_exchange(
        &self,
        current: Option<CpuId>,
        new: Option<CpuId>,
        success: Ordering,
        failure: Ordering,
    ) -> Result<Option<CpuId>, Option<CpuId>> {
        let current_val = Self::val_from_id(current);
        let new_val = Self::val_from_id(new);
        match self
            .inner
            .compare_exchange(current_val, new_val, success, failure)
        {
            Ok(_) => Ok(Self::id_from_val(current_val)),
            Err(val) => Err(Self::id_from_val(val)),
        }
    }

    const fn val_from_id(id: Option<CpuId>) -> u32 {
        match id {
            Some(id) => id.0,
            None => Self::NONE_VALUE,
        }
    }

    const fn id_from_val(val: u32) -> Option<CpuId> {
        if val == Self::NONE_VALUE {
            None
        } else {
            Some(CpuId(val))
        }
    }
}
