// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicU32, Ordering};

use atomic_integer_wrapper::define_atomic_version_of_integer_like_type;

use crate::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Pod)]
#[repr(C)]
pub struct Securebits(u32);

const DEFAULT_SECUREBITS: u32 = 0;

impl Securebits {
    pub const fn default() -> Self {
        Self(DEFAULT_SECUREBITS)
    }

    pub const fn new(securebits: u32) -> Self {
        Self(securebits)
    }
}

impl From<u32> for Securebits {
    fn from(value: u32) -> Self {
        Self::new(value)
    }
}

impl From<Securebits> for u32 {
    fn from(value: Securebits) -> Self {
        value.0
    }
}

define_atomic_version_of_integer_like_type!(Securebits, {
    #[derive(Debug)]
    pub(super) struct AtomicSecurebits(AtomicU32);
});

impl Clone for AtomicSecurebits {
    fn clone(&self) -> Self {
        Self::new(self.load(Ordering::Acquire))
    }
}
