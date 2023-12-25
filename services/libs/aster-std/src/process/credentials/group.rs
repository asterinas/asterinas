use core::sync::atomic::{AtomicU32, Ordering};

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

    pub const fn as_u32(&self) -> u32 {
        self.0
    }

    pub const fn is_root(&self) -> bool {
        self.0 == ROOT_GID
    }
}

const ROOT_GID: u32 = 0;

#[derive(Debug)]
pub(super) struct AtomicGid(AtomicU32);

impl AtomicGid {
    pub const fn new(gid: Gid) -> Self {
        Self(AtomicU32::new(gid.as_u32()))
    }

    pub fn set(&self, gid: Gid) {
        self.0.store(gid.as_u32(), Ordering::Relaxed)
    }

    pub fn get(&self) -> Gid {
        Gid(self.0.load(Ordering::Relaxed))
    }
}

impl Clone for AtomicGid {
    fn clone(&self) -> Self {
        Self(AtomicU32::new(self.0.load(Ordering::Relaxed)))
    }
}
