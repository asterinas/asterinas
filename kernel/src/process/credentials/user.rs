// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicU32, Ordering};

use atomic_integer_wrapper::define_atomic_version_of_integer_like_type;

use crate::prelude::*;

/// The raw UID type at the syscall ABI boundary, matching Linux's `uid_t`.
pub type RawUid = u32;

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, Pod)]
pub struct Uid(u32);

const ROOT_UID: u32 = 0;

impl Uid {
    /// The raw value representing an invalid UID (`(uid_t)-1` in Linux).
    ///
    /// Reference: <https://elixir.bootlin.com/linux/v6.15/source/include/linux/uidgid.h#L50>.
    pub const RAW_INVALID: RawUid = u32::MAX;

    /// The overflow UID, typically used to indicate that user mappings between namespaces fail.
    ///
    /// This is currently a constant (65534 is usually the "nobody" user), but it should be
    /// configured via `/proc/sys/kernel/overflowuid`.
    ///
    /// Reference: <https://elixir.bootlin.com/linux/v6.15/source/kernel/sys.c#L166>.
    pub const OVERFLOW: Uid = Self(65534);

    pub const fn new_root() -> Self {
        Self(ROOT_UID)
    }

    /// Creates a `Uid` from a raw value, returning `None` if the value is the
    /// invalid sentinel (i.e., [`Self::RAW_INVALID`]).
    pub const fn new(uid: RawUid) -> Option<Self> {
        if uid == Self::RAW_INVALID {
            None
        } else {
            Some(Self(uid))
        }
    }

    /// Creates a `Uid` from a raw value without checking for the invalid sentinel.
    ///
    /// This is intended for filesystem use where any raw UID stored on disk is valid.
    pub const fn from_raw(uid: RawUid) -> Self {
        Self(uid)
    }

    /// Returns whether this UID has a valid mapping (i.e., is not the invalid sentinel).
    pub const fn has_valid_mapping(&self) -> bool {
        self.0 != Self::RAW_INVALID
    }

    /// Returns the underlying raw UID value.
    pub const fn as_raw(&self) -> RawUid {
        self.0
    }

    pub const fn is_root(&self) -> bool {
        self.0 == ROOT_UID
    }
}

impl From<Uid> for u32 {
    fn from(value: Uid) -> Self {
        value.0
    }
}

impl TryFrom<u32> for Uid {
    type Error = Error;

    fn try_from(value: u32) -> Result<Self> {
        Self::new(value).ok_or_else(|| Error::with_message(Errno::EINVAL, "the UID is invalid"))
    }
}

define_atomic_version_of_integer_like_type!(Uid, try_from = true, {
    #[derive(Debug)]
    pub(super) struct AtomicUid(AtomicU32);
});

impl Clone for AtomicUid {
    fn clone(&self) -> Self {
        Self::new(self.load(Ordering::Relaxed))
    }
}
