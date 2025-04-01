// SPDX-License-Identifier: MPL-2.0

use super::{
    cursor::Cursor,
    mark::{NoneMark, XMark},
};
use crate::sync::non_null::NonNullPtr;

/// An iterator over a range of entries in an [`XArray`].
///
/// The typical way to obtain a `Range` instance is to call [`XArray::range`].
///
/// [`XArray`]: super::XArray
/// [`XArray::range`]: super::XArray::range
pub struct Range<'a, P, M = NoneMark>
where
    P: NonNullPtr + Sync,
    M: Into<XMark>,
{
    cursor: Cursor<'a, P, M>,
    end: u64,
}

impl<'a, P: NonNullPtr + Sync, M: Into<XMark>> Range<'a, P, M> {
    pub(super) fn new(cursor: Cursor<'a, P, M>, end: u64) -> Self {
        Range { cursor, end }
    }
}

impl<'a, P: NonNullPtr + Sync, M: Into<XMark>> core::iter::Iterator for Range<'a, P, M> {
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
