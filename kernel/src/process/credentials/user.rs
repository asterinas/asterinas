// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicU32, Ordering};

use atomic_integer_wrapper::define_atomic_version_of_integer_like_type;

use crate::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Pod)]
#[repr(C)]
pub struct Uid(u32);

const ROOT_UID: u32 = 0;

impl Uid {
    pub const fn new_root() -> Self {
        Self(ROOT_UID)
    }

    pub const fn new(uid: u32) -> Self {
        Self(uid)
    }

    pub const fn is_root(&self) -> bool {
        self.0 == ROOT_UID
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

impl AtomicUid {
    pub fn is_root(&self) -> bool {
        self.load(Ordering::Acquire).is_root()
    }
}

impl Clone for AtomicUid {
    fn clone(&self) -> Self {
        Self::new(self.load(Ordering::Acquire))
    }
}
