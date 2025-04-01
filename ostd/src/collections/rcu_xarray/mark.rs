// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicU64, Ordering};

/// A mark used to indicate which slots in an [`XNode`] contain items that have been marked.
///
/// [`Xnode`]: super::node::XNode
///
/// It internally stores a `AtomicU64`, functioning as a bitmap, where each bit that is set to 1
/// represents a slot at the corresponding offset that has been marked.
#[derive(Debug)]
pub(super) struct Mark {
    inner: AtomicU64,
}

#[expect(clippy::declare_interior_mutable_const)]
impl Mark {
    pub const EMPTY: Self = Self::new(0);

    pub const fn new(inner: u64) -> Self {
        Self {
            inner: AtomicU64::new(inner),
        }
    }

    pub fn update(&self, offset: u8, set: bool) -> bool {
        let val = self.inner.load(Ordering::Acquire);
        let new_val = if set {
            val | (1 << offset as u64)
        } else {
            val & !(1 << offset as u64)
        };

        let old_val = self.inner.swap(new_val, Ordering::AcqRel);

        old_val != new_val
    }

    pub fn is_marked(&self, offset: u8) -> bool {
        self.inner.load(Ordering::Acquire) & (1 << offset as u64) != 0
    }

    pub fn is_clear(&self) -> bool {
        self.inner.load(Ordering::Acquire) == 0
    }
}

/// The mark type used in the [`XArray`].
///
/// The `XArray` itself and an item in it can have up to three different marks.
///
/// Users can use a self-defined type to distinguish which kind of mark they want to set. Such a
/// type must implement the `Into<XMark>` trait.
///
/// [`XArray`]: crate::XArray
pub enum XMark {
    /// The mark kind 0.
    Mark0,
    /// The mark kind 1.
    Mark1,
    /// The mark kind 2.
    Mark2,
}

pub const NUM_MARKS: usize = 3;

impl XMark {
    /// Maps the `XMark` to an index in the range 0 to 2.
    pub(super) fn index(&self) -> usize {
        match self {
            XMark::Mark0 => 0,
            XMark::Mark1 => 1,
            XMark::Mark2 => 2,
        }
    }
}

/// A meaningless mark used as a default generic parameter for `XArray`, indicating that the
/// marking functionality is not needed.
#[derive(Clone, Copy)]
pub struct NoneMark {}

impl From<NoneMark> for XMark {
    fn from(_val: NoneMark) -> Self {
        panic!("NoneMark can not be used!");
    }
}
