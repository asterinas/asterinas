// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::AtomicU32;

use atomic_integer_wrapper::define_atomic_version_of_integer_like_type;
use bitflags::bitflags;

bitflags! {
    pub struct StatusFlags: u32 {
        /// append on each write
        const O_APPEND = 1 << 10;
        /// non block
        const O_NONBLOCK = 1 << 11;
        /// synchronized I/O, data
        const O_DSYNC = 1 << 12;
        /// signal-driven I/O
        const O_ASYNC = 1 << 13;
        /// direct I/O
        const O_DIRECT = 1 << 14;
        /// on x86_64, O_LARGEFILE is 0
        /// not update st_atime
        const O_NOATIME = 1 << 18;
        /// synchronized I/O, data and metadata
        const O_SYNC = 1 << 20;
        /// equivalent of POSIX.1's O_EXEC
        const O_PATH = 1 << 21;
    }
}

impl StatusFlags {
    /// Status flags that can be changed by `F_SETFL`.
    pub const SETFL_MASK: Self = Self::O_APPEND
        .union(Self::O_ASYNC)
        .union(Self::O_DIRECT)
        .union(Self::O_NOATIME)
        .union(Self::O_NONBLOCK);
}

/// Status flags that a file supports changing after it is opened.
///
/// Every value contains `O_APPEND`, `O_NOATIME`, and `O_NONBLOCK`. Files may additionally support
/// changing `O_ASYNC` and `O_DIRECT`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SettableStatusFlags(StatusFlags);

impl SettableStatusFlags {
    /// Creates the minimal set of status flags supported by all files.
    pub const fn minimal() -> Self {
        Self(
            StatusFlags::O_APPEND
                .union(StatusFlags::O_NOATIME)
                .union(StatusFlags::O_NONBLOCK),
        )
    }

    /// Adds support for changing `O_ASYNC`.
    pub const fn with_o_async(self) -> Self {
        Self(self.0.union(StatusFlags::O_ASYNC))
    }

    /// Adds support for changing `O_DIRECT`.
    pub const fn with_o_direct(self) -> Self {
        Self(self.0.union(StatusFlags::O_DIRECT))
    }

    pub(in crate::fs::file) fn contains(self, flags: StatusFlags) -> bool {
        self.0.contains(flags)
    }
}

impl From<u32> for StatusFlags {
    fn from(value: u32) -> Self {
        Self::from_bits_truncate(value)
    }
}

impl From<StatusFlags> for u32 {
    fn from(value: StatusFlags) -> Self {
        value.bits()
    }
}

define_atomic_version_of_integer_like_type!(StatusFlags, {
    /// An atomic version of `StatusFlags`.
    #[derive(Debug)]
    pub struct AtomicStatusFlags(AtomicU32);
});
