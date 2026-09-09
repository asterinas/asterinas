// SPDX-License-Identifier: MPL-2.0

//! Small helpers shared across the ext2 module.
//!
//! - `Dirty` — a wrapper that tracks whether its inner value has been
//!   mutated, for writeback scheduling.
//! - `IsPowerOf` — a trait for testing whether a number is a power of
//!   another (used by sparse-superblock backup-group selection).
//! - `now` — current wall-clock time as a VFS [`UnixTimestamp`].
//! - `now_duration` — the same instant as a [`Duration`], for superblock fields.
//! - `duration_to_ext2_secs` — converts kernel durations to clamped ext2
//!   timestamp seconds.
//! - `ext2_secs_to_unix_timestamp` and `unix_timestamp_to_ext2_secs` — convert
//!   between VFS timestamps and ext2's signed 32-bit seconds format.
//! - `normalize_ext2_timestamp` — clamps a VFS timestamp to the range and
//!   precision representable by ext2.

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

/// Returns the current wall-clock time as a VFS timestamp.
pub(super) fn now() -> UnixTimestamp {
    UnixTimestamp::from_duration_since_epoch(now_duration())
}

/// Returns the current wall-clock time as a `Duration` since the Unix epoch.
///
/// Superblock fields still use unsigned `UnixTime` / `Duration`.
pub(super) fn now_duration() -> Duration {
    crate::time::clocks::RealTimeCoarseClock::get().read_time()
}

/// Converts a `Duration` to a 32-bit ext2 timestamp, clamping to `u32::MAX`.
pub(super) fn duration_to_ext2_secs(d: Duration) -> u32 {
    u32::try_from(d.as_secs()).unwrap_or(u32::MAX)
}

/// Decodes an ext2 inode timestamp from signed 32-bit seconds.
///
/// See the original inode timestamp format in
/// <https://www.kernel.org/doc/html/latest/filesystems/ext4/inodes.html#inode-timestamps>.
pub(super) fn ext2_secs_to_unix_timestamp(seconds: u32) -> UnixTimestamp {
    UnixTimestamp::from_seconds(seconds as i32 as i64)
}

/// Encodes a timestamp using ext2's signed 32-bit seconds format.
pub(super) fn unix_timestamp_to_ext2_secs(ts: UnixTimestamp) -> u32 {
    ts.seconds().clamp(i32::MIN as i64, i32::MAX as i64) as u32
}

/// Clamps a timestamp to the range and precision representable by ext2.
pub(super) fn normalize_ext2_timestamp(ts: UnixTimestamp) -> UnixTimestamp {
    ext2_secs_to_unix_timestamp(unix_timestamp_to_ext2_secs(ts))
}

#[cfg(ktest)]
mod tests {
    use ostd::prelude::*;

    use super::*;

    #[ktest]
    fn encode_negative_one_is_all_ones() {
        let encoded = unix_timestamp_to_ext2_secs(UnixTimestamp::from_seconds(-1));
        assert_eq!(encoded, u32::MAX);
        assert_eq!(ext2_secs_to_unix_timestamp(encoded).seconds(), -1);
    }

    #[ktest]
    fn encode_clamps_to_signed_32() {
        assert_eq!(
            unix_timestamp_to_ext2_secs(UnixTimestamp::from_seconds(i32::MAX as i64 + 1)),
            i32::MAX as u32
        );
        assert_eq!(
            unix_timestamp_to_ext2_secs(UnixTimestamp::from_seconds(i32::MIN as i64 - 1)),
            i32::MIN as u32
        );
    }

    #[ktest]
    fn normalize_clamps_seconds_and_discards_nanoseconds() {
        assert_eq!(
            normalize_ext2_timestamp(UnixTimestamp::try_new(123, 456).unwrap()),
            UnixTimestamp::from_seconds(123)
        );
        assert_eq!(
            normalize_ext2_timestamp(UnixTimestamp::try_new(i32::MAX as i64 + 1, 123).unwrap()),
            UnixTimestamp::from_seconds(i32::MAX as i64)
        );
        assert_eq!(
            normalize_ext2_timestamp(UnixTimestamp::try_new(i32::MIN as i64 - 1, 456).unwrap()),
            UnixTimestamp::from_seconds(i32::MIN as i64)
        );
    }
}
