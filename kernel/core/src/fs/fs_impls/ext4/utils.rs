// SPDX-License-Identifier: MPL-2.0

//! Small helpers shared across the ext2 module.
//!
//! - `Dirty` — a wrapper that tracks whether its inner value has been
//!   mutated, for writeback scheduling.
//! - `IsPowerOf` — a trait for testing whether a number is a power of
//!   another (used by sparse-superblock backup-group selection).
//! - `now` — a convenience function that reads the real-time coarse clock.
//! - `duration_to_ext2_secs` — converts kernel durations to clamped ext2
//!   timestamp seconds.

use core::ops::MulAssign;

use super::prelude::*;
use crate::prelude::warn;

pub(super) trait IsPowerOf: Copy + Sized + MulAssign + PartialOrd {
    /// Returns whether `self` equals `x^k` for some `k > 0`.
    ///
    /// `x` must be greater than 1.
    fn is_power_of(&self, x: Self) -> bool {
        let mut power = x;
        while power < *self {
            power *= x;
        }

        power == *self
    }
}

macro_rules! impl_ipo_for {
    ($($ipo_ty:ty),*) => {
        $(impl IsPowerOf for $ipo_ty {})*
    };
}

impl_ipo_for!(
    u8, u16, u32, u64, u128, i8, i16, i32, i64, i128, isize, usize
);

/// A value with dirty tracking.
pub(super) struct Dirty<T: Debug> {
    value: T,
    dirty: bool,
}

impl<T: Debug> Dirty<T> {
    /// Creates a new `Dirty` value without setting the dirty flag.
    pub(super) fn new(val: T) -> Dirty<T> {
        Dirty {
            value: val,
            dirty: false,
        }
    }

    /// Creates a new `Dirty` value with the dirty flag set.
    pub(super) fn _new_dirty(val: T) -> Dirty<T> {
        Dirty {
            value: val,
            dirty: true,
        }
    }

    /// Returns whether the value is dirty.
    pub(super) fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Clears the dirty flag.
    pub(super) fn clear_dirty(&mut self) {
        self.dirty = false;
    }
}

impl<T: Debug> Deref for Dirty<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.value
    }
}

impl<T: Debug> DerefMut for Dirty<T> {
    fn deref_mut(&mut self) -> &mut T {
        self.dirty = true;
        &mut self.value
    }
}

impl<T: Debug> Drop for Dirty<T> {
    fn drop(&mut self) {
        if self.is_dirty() {
            warn!("dropped while dirty: {:?}", self.value);
        }
    }
}

impl<T: Debug> Debug for Dirty<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let tag = if self.dirty { "Dirty" } else { "Clean" };
        write!(f, "[{}] {:?}", tag, self.value)
    }
}

/// Returns the current time.
pub(super) fn now() -> Duration {
    crate::time::clocks::RealTimeCoarseClock::get().read_time()
}

/// Converts a `Duration` to a 32-bit ext2 timestamp, clamping to `u32::MAX`.
pub(super) fn duration_to_ext2_secs(d: Duration) -> u32 {
    u32::try_from(d.as_secs()).unwrap_or(u32::MAX)
}
