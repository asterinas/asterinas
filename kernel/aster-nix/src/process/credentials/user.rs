// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicU32, Ordering};

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

    pub const fn as_u32(&self) -> u32 {
        self.0
    }
}

#[derive(Debug)]
pub(super) struct AtomicUid(AtomicU32);

impl AtomicUid {
    pub const fn new(uid: Uid) -> Self {
        Self(AtomicU32::new(uid.as_u32()))
    }

    pub fn set(&self, uid: Uid) {
        self.0.store(uid.as_u32(), Ordering::Release)
    }

    pub fn get(&self) -> Uid {
        Uid(self.0.load(Ordering::Acquire))
    }

    pub fn is_root(&self) -> bool {
        self.get().is_root()
    }
}

impl Clone for AtomicUid {
    fn clone(&self) -> Self {
        Self(AtomicU32::new(self.0.load(Ordering::Acquire)))
    }
}
