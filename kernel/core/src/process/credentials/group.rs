// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicU32, Ordering};

use atomic_integer_wrapper::define_atomic_version_of_integer_like_type;

use crate::prelude::*;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd, Pod)]
pub(crate) struct Gid(u32);

/// The default overflow GID, which is the `nogroup` group (`65534`).
///
/// Reference: <https://elixir.bootlin.com/linux/v6.15/source/include/linux/highuid.h#L42>.
const DEFAULT_OVERFLOW_GID: u32 = 65534;

/// The overflow GID, typically used to indicate that group mappings between namespaces fail.
/// The value can be configured via `/proc/sys/kernel/overflowgid`.
static OVERFLOW_GID: AtomicU32 = AtomicU32::new(DEFAULT_OVERFLOW_GID);

impl Gid {
    /// The invalid GID, typically used to indicate that no valid GID is found when returning to user space.
    ///
    /// Reference: <https://elixir.bootlin.com/linux/v6.15/source/include/linux/uidgid.h#L51>.
    pub(crate) const INVALID: Gid = Gid(u32::MAX);

    pub(crate) fn overflow() -> Self {
        Self::new(OVERFLOW_GID.load(Ordering::Relaxed))
    }

    pub(crate) fn set_overflow(gid: Self) {
        OVERFLOW_GID.store(gid.0, Ordering::Relaxed);
    }

    pub(crate) const fn new(gid: u32) -> Self {
        Self(gid)
    }

    pub(crate) const fn new_root() -> Self {
        Self(ROOT_GID)
    }

    pub(crate) const fn is_root(&self) -> bool {
        self.0 == ROOT_GID
    }
}

const ROOT_GID: u32 = 0;

impl From<u32> for Gid {
    fn from(value: u32) -> Self {
        Self::new(value)
    }
}

impl From<Gid> for u32 {
    fn from(value: Gid) -> Self {
        value.0
    }
}

define_atomic_version_of_integer_like_type!(Gid, {
    #[derive(Debug)]
    pub(super) struct AtomicGid(AtomicU32);
});

impl Clone for AtomicGid {
    fn clone(&self) -> Self {
        Self::new(self.load(Ordering::Relaxed))
    }
}
