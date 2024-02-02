// SPDX-License-Identifier: MPL-2.0

//! Intervals and interval sets used in VMARs.

use core::ops::Range;

/// An interval is associated with a range of values (`T`).
pub trait Interval<T> {
    /// Returns the range of the interval.
    fn range(&self) -> Range<T>;
}

/// A collection that contains intervals as items. In particular,
/// the collection allows one to retrieve interval items that intersect with
/// a point of value or range of values.
pub trait IntervalSet<'a, T> {
    type Item: Interval<T> + 'a;

    /// Find the interval items that overlap with a specific range.
    fn find(&'a self, range: &Range<T>) -> impl IntoIterator<Item = &'a Self::Item> + 'a;

    /// Finds one interval item that contains the point.
    ///
    /// If there are multiple such items, then an arbitrary one is returned.
    fn find_one(&'a self, point: &T) -> Option<&'a Self::Item>;
}
