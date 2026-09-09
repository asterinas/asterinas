// SPDX-License-Identifier: MPL-2.0

//! Signed Unix timestamps for VFS metadata.
//!
//! [`UnixTimestamp`] represents wall-clock instants with signed seconds
//! and normalized nanoseconds.
//! It bridges real-time clocks, syscall time structures, and filesystem metadata
//! without using [`Duration`] for stored timestamps.

use core::time::Duration;

/// Signed Unix time, used for VFS inode timestamps.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct UnixTimestamp {
    seconds: i64,
    nanoseconds: u32,
}

impl UnixTimestamp {
    const UNIX_EPOCH: Self = Self {
        seconds: 0,
        nanoseconds: 0,
    };

    const MAX: Self = Self {
        seconds: i64::MAX,
        nanoseconds: super::NSEC_PER_SEC as u32 - 1,
    };

    pub(crate) const fn from_seconds(seconds: i64) -> Self {
        Self {
            seconds,
            nanoseconds: 0,
        }
    }

    /// Creates a timestamp if `nanoseconds` is normalized.
    ///
    /// Use this constructor for timestamps supplied by untrusted sources,
    /// such as userspace or FUSE.
    pub(crate) const fn try_new(seconds: i64, nanoseconds: u32) -> Option<Self> {
        if nanoseconds < super::NSEC_PER_SEC as u32 {
            Some(Self {
                seconds,
                nanoseconds,
            })
        } else {
            None
        }
    }

    pub(crate) fn seconds(&self) -> i64 {
        self.seconds
    }

    pub(crate) fn nanoseconds(&self) -> u32 {
        self.nanoseconds
    }

    /// Converts a duration since the Unix epoch into a timestamp.
    pub(crate) fn from_duration_since_epoch(duration: Duration) -> Self {
        let Ok(seconds) = i64::try_from(duration.as_secs()) else {
            return Self::MAX;
        };

        Self {
            seconds,
            nanoseconds: duration.subsec_nanos(),
        }
    }
}

impl Default for UnixTimestamp {
    fn default() -> Self {
        Self::UNIX_EPOCH
    }
}

#[cfg(ktest)]
mod tests {
    use ostd::prelude::*;

    use super::*;

    #[ktest]
    fn try_new_accepts_negative_seconds() {
        let ts = UnixTimestamp::try_new(-1, 0).unwrap();
        assert_eq!(ts.seconds(), -1);
        assert_eq!(ts.nanoseconds(), 0);
    }

    #[ktest]
    fn try_new_rejects_unnormalized_nsec() {
        assert!(UnixTimestamp::try_new(0, 1_000_000_000).is_none());
    }

    #[ktest]
    fn from_duration_preserves_subsec() {
        let ts = UnixTimestamp::from_duration_since_epoch(Duration::from_nanos(1_000_000_042));
        assert_eq!(ts.seconds(), 1);
        assert_eq!(ts.nanoseconds(), 42);
    }

    #[ktest]
    fn from_duration_saturates_unrepresentable_seconds() {
        let duration = Duration::new(i64::MAX as u64 + 1, 0);
        let ts = UnixTimestamp::from_duration_since_epoch(duration);
        assert_eq!(ts.seconds(), i64::MAX);
        assert_eq!(ts.nanoseconds(), 999_999_999);
    }

    #[ktest]
    fn timespec_roundtrip_allows_negative_seconds() {
        let spec = crate::time::timespec_t { sec: -1, nsec: 123 };
        let ts = UnixTimestamp::try_from(spec).unwrap();
        assert_eq!(ts.seconds(), -1);
        assert_eq!(ts.nanoseconds(), 123);
        let back = crate::time::timespec_t::from(ts);
        assert_eq!(back.sec, -1);
        assert_eq!(back.nsec, 123);
    }

    #[ktest]
    fn timespec_rejects_bad_nsec_but_not_negative_sec() {
        use crate::time::timespec_t;

        assert!(UnixTimestamp::try_from(timespec_t { sec: -5, nsec: 0 }).is_ok());
        assert!(UnixTimestamp::try_from(timespec_t { sec: -5, nsec: -1 }).is_err());
        assert!(
            UnixTimestamp::try_from(timespec_t {
                sec: -5,
                nsec: 1_000_000_000,
            })
            .is_err()
        );
    }

    #[ktest]
    fn duration_try_from_still_rejects_negative_sec() {
        use crate::time::timespec_t;

        assert!(Duration::try_from(timespec_t { sec: -1, nsec: 0 }).is_err());
        assert!(Duration::try_from(timespec_t { sec: 1, nsec: 0 }).is_ok());
    }

    // FUSE/ext2 on-disk seconds are `u64`/`u32` bit patterns of signed Unix time.
    #[ktest]
    fn two_complement_bit_patterns_decode_as_signed() {
        assert_eq!(u32::MAX as i32 as i64, -1);
        assert_eq!(u64::MAX as i64, -1);
        assert_eq!((i32::MIN as u32) as i32 as i64, i32::MIN as i64);
    }
}
