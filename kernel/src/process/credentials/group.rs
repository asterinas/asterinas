// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicU32, Ordering};

use atomic_integer_wrapper::define_atomic_version_of_integer_like_type;

use crate::prelude::*;

#[derive(Debug, Clone, Copy, Pod, Default, PartialEq, Eq, PartialOrd, Ord)]
#[repr(C)]
pub struct Gid(u32);

impl Gid {
    pub const fn new(gid: u32) -> Self {
        Self(gid)
    }

    pub const fn new_root() -> Self {
        Self(ROOT_GID)
    }

    pub const fn is_root(&self) -> bool {
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
