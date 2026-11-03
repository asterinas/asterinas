// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicU32, Ordering};

use atomic_integer_wrapper::define_atomic_version_of_integer_like_type;

use crate::prelude::*;

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, Pod)]
pub(crate) struct Uid(u32);

const ROOT_UID: u32 = 0;

/// The default overflow UID, which is the `nobody` user (`65534`).
///
/// Reference: <https://elixir.bootlin.com/linux/v6.15/source/include/linux/highuid.h#L41>.
const DEFAULT_OVERFLOW_UID: u32 = 65534;

/// The overflow UID, typically used to indicate that user mappings between namespaces fail.
/// The value can be configured via `/proc/sys/kernel/overflowuid`.
static OVERFLOW_UID: AtomicU32 = AtomicU32::new(DEFAULT_OVERFLOW_UID);

impl Uid {
    /// The invalid UID, typically used to indicate that no valid UID is found when returning to user space.
    ///
    /// Reference: <https://elixir.bootlin.com/linux/v6.15/source/include/linux/uidgid.h#L50>.
    pub(crate) const INVALID: Uid = Self::new(u32::MAX);

    pub(crate) const fn new(uid: u32) -> Self {
        Self(uid)
    }

    pub(crate) const fn new_root() -> Self {
        Self(ROOT_UID)
    }

    pub(crate) const fn is_root(&self) -> bool {
        self.0 == ROOT_UID
    }

    pub(crate) fn new_overflow() -> Self {
        Self::new(OVERFLOW_UID.load(Ordering::Relaxed))
    }

    pub(crate) fn set_overflow(uid: Self) {
        OVERFLOW_UID.store(uid.0, Ordering::Relaxed);
    }
}

impl From<u32> for Uid {
    fn from(value: u32) -> Self {
        Self::new(value)
    }
}

impl From<Uid> for u32 {
    fn from(value: Uid) -> Self {
        value.0
    }
}

define_atomic_version_of_integer_like_type!(Uid, {
    #[derive(Debug)]
    pub(super) struct AtomicUid(AtomicU32);
});

impl Clone for AtomicUid {
    fn clone(&self) -> Self {
        Self::new(self.load(Ordering::Relaxed))
    }
}
