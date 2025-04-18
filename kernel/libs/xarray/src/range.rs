// SPDX-License-Identifier: MPL-2.0

use ostd::{
    sync::non_null::NonNullPtr,
    task::{atomic_mode::AsAtomicModeGuard, DisabledPreemptGuard},
};

use crate::{cursor::Cursor, mark::NoneMark};

/// An iterator over a range of entries in an [`XArray`].
///
/// The typical way to obtain a `Range` instance is to call [`XArray::range`].
///
/// [`XArray`]: super::XArray
/// [`XArray::range`]: super::XArray::range
pub struct Range<'a, P, M = NoneMark, G = DisabledPreemptGuard>
where
    P: NonNullPtr + Send + Sync,
{
    cursor: Cursor<'a, P, M, G>,
    end: u64,
}

impl<'a, P: NonNullPtr + Send + Sync, M, G: AsAtomicModeGuard> Range<'a, P, M, G> {
    pub(super) fn new(cursor: Cursor<'a, P, M, G>, end: u64) -> Self {
        Range { cursor, end }
    }
}

impl<'a, P: NonNullPtr + Send + Sync, M, G: AsAtomicModeGuard> core::iter::Iterator
    for Range<'a, P, M, G>
{
    type Item = (u64, P::Ref<'a>);

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.cursor.index() >= self.end {
                return None;
            }

            let item = self.cursor.load();
            if item.is_none() {
                self.cursor.next();
                continue;
            }

            let res = item.map(|item| (self.cursor.index(), item));
            self.cursor.next();
            return res;
        }
    }
}
