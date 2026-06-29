// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicU32, Ordering};

use atomic_integer_wrapper::define_atomic_version_of_integer_like_type;

use crate::prelude::*;

/// The raw GID type at the syscall ABI boundary, matching Linux's `gid_t`.
pub type RawGid = u32;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd, Pod)]
pub struct Gid(u32);

impl Gid {
    /// The raw value representing an invalid GID (`(gid_t)-1` in Linux).
    ///
    /// Reference: <https://elixir.bootlin.com/linux/v6.15/source/include/linux/uidgid.h#L51>.
    pub const RAW_INVALID: RawGid = u32::MAX;

    /// The overflow GID, typically used to indicate that group mappings between namespaces fail.
    ///
    /// This is currently a constant (65534 is usually the "nobody" group), but it should be
    /// configured via `/proc/sys/kernel/overflowgid`.
    ///
    /// Reference: <https://elixir.bootlin.com/linux/v6.15/source/kernel/sys.c#L167>.
    pub const OVERFLOW: Gid = Self(65534);

    /// Creates a `Gid` from a raw value, returning `None` if the value is the
    /// invalid sentinel (i.e., [`Self::RAW_INVALID`]).
    pub const fn new(gid: RawGid) -> Option<Self> {
        if gid == Self::RAW_INVALID {
            None
        } else {
            Some(Self(gid))
        }
    }

    /// Creates a `Gid` from a raw value without checking for the invalid sentinel.
    ///
    /// This is intended for filesystem use where any raw GID stored on disk is valid.
    pub const fn from_raw(gid: RawGid) -> Self {
        Self(gid)
    }

    /// Returns whether this GID has a valid mapping (i.e., is not the invalid sentinel).
    pub const fn has_valid_mapping(&self) -> bool {
        self.0 != Self::RAW_INVALID
    }

    pub const fn new_root() -> Self {
        Self(ROOT_GID)
    }

    /// Returns the underlying raw GID value.
    pub const fn as_raw(&self) -> RawGid {
        self.0
    }

    pub const fn is_root(&self) -> bool {
        self.0 == ROOT_GID
    }
}

const ROOT_GID: u32 = 0;

impl From<Gid> for u32 {
    fn from(value: Gid) -> Self {
        value.0
    }
}

impl TryFrom<u32> for Gid {
    type Error = Error;

    fn try_from(value: u32) -> Result<Self> {
        Self::new(value).ok_or_else(|| Error::with_message(Errno::EINVAL, "the GID is invalid"))
    }
}

define_atomic_version_of_integer_like_type!(Gid, try_from = true, {
    #[derive(Debug)]
    pub(super) struct AtomicGid(AtomicU32);
});

impl Clone for AtomicGid {
    fn clone(&self) -> Self {
        Self::new(self.load(Ordering::Relaxed))
    }
}
