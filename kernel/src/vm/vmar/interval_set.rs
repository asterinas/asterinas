// SPDX-License-Identifier: MPL-2.0

//! Intervals and interval sets used in VMARs.

use alloc::collections::btree_map::{BTreeMap, Cursor, CursorMut};
use core::ops::Range;

/// The interval of an item in an interval set.
///
/// All items in the interval set must have a range.
pub trait Interval<K: Clone> {
    /// Returns the range of the interval.
    fn range(&self) -> Range<K>;
}

/// A collection that contains non-overlapping intervals as items.
///
/// In particular, the collection allows one to retrieve interval items that
/// intersect with a point of value or range of values.
#[derive(Debug)]
pub struct IntervalSet<K, V>
where
    K: Clone + Ord,
    V: Interval<K>,
{
    btree: BTreeMap<K, V>,
}

impl<K, V> Default for IntervalSet<K, V>
where
    K: Clone + Ord,
    V: Interval<K>,
{
    fn default() -> Self {
        Self::new()
    }
}

#[allow(dead_code)]
impl<K, V> IntervalSet<K, V>
where
    K: Clone + Ord,
    V: Interval<K>,
{
    /// Creates a new interval set.
    pub const fn new() -> Self {
        Self {
            btree: BTreeMap::new(),
        }
    }

    /// Inserts an interval item into the interval set.
    pub fn insert(&mut self, item: V) {
        let range = item.range();
        self.btree.insert(range.start, item);
    }

    /// Removes an interval item from the interval set.
    pub fn remove(&mut self, key: &K) -> Option<V> {
        self.btree.remove(key)
    }

    /// Returns an iterator over the interval items in the interval set.
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &V> {
        self.btree.values()
    }

    /// Finds an interval item that contains the given point.
    ///
    /// If no such item exists, returns [`None`]. Otherwise, returns the item
    /// that contains the point.
    pub fn find_one(&self, point: &K) -> Option<&V> {
        let cursor = self.btree.lower_bound(core::ops::Bound::Excluded(point));
        // There's one previous element and one following element that may
        // contain the point. If they don't, there's no other chances.
        if let Some((_, v)) = cursor.peek_prev() {
            if v.range().end > *point {
                return Some(v);
            }
        } else if let Some((_, v)) = cursor.peek_next() {
            if v.range().start <= *point {
                return Some(v);
            }
        }
        None
    }

    /// Finds all interval items that intersect with the given range.
    pub fn find<'a>(&'a self, range: &Range<K>) -> IntervalIter<'a, K, V> {
        let cursor = self
            .btree
            .lower_bound(core::ops::Bound::Excluded(&range.start));
        IntervalIter {
            cursor,
            range: range.clone(),
            peeked_prev: false,
        }
    }

    /// Takes an interval item that contains the given point.
    ///
    /// If no such item exists, returns [`None`]. Otherwise, returns the item
    /// that contains the point.
    pub fn take_one(&mut self, point: &K) -> Option<V> {
        let mut cursor = self
            .btree
            .lower_bound_mut(core::ops::Bound::Excluded(point));
        // There's one previous element and one following element that may
        // contain the point. If they don't, there's no other chances.
        if let Some((_, v)) = cursor.peek_prev() {
            if v.range().end > *point {
                return Some(cursor.remove_prev().unwrap().1);
            }
        } else if let Some((_, v)) = cursor.peek_next() {
            if v.range().start <= *point {
                return Some(cursor.remove_next().unwrap().1);
            }
        }
        None
    }

    /// Takes all interval items that intersect with the given range.
    ///
    /// This method returns a draining iterator that removes the items from the
    /// interval set.
    pub fn take<'a>(&'a mut self, range: &Range<K>) -> IntervalDrain<'a, K, V> {
        let cursor = self
            .btree
            .lower_bound_mut(core::ops::Bound::Excluded(&range.start));
        IntervalDrain {
            cursor,
            range: range.clone(),
            drained_prev: false,
        }
    }

    /// Clears the interval set, removing all intervals.
    pub fn clear(&mut self) {
        self.btree.clear();
    }
}

/// An iterator that iterates over intervals in an interval set.
#[derive(Debug)]
pub struct IntervalIter<'a, K, V>
where
    K: Clone + Ord,
    V: Interval<K>,
{
    cursor: Cursor<'a, K, V>,
    range: Range<K>,
    peeked_prev: bool,
}

impl<'a, K, V> Iterator for IntervalIter<'a, K, V>
where
    K: Clone + Ord,
    V: Interval<K>,
{
    type Item = &'a V;

    fn next(&mut self) -> Option<Self::Item> {
        // There's one previous element that may intersect with the range.
        if !self.peeked_prev {
            self.peeked_prev = true;
            if let Some((_, v)) = self.cursor.peek_prev() {
                if v.range().end > self.range.start {
                    return Some(v);
                }
            }
        }

        // Find all intersected elements following it.
        if let Some((_, v)) = self.cursor.next() {
            if v.range().start >= self.range.end {
                return None;
            }
            return Some(v);
        }

        None
    }
}

/// A draining iterator that iterates over intervals in an interval set.
#[derive(Debug)]
pub struct IntervalDrain<'a, K, V>
where
    K: Clone + Ord,
    V: Interval<K>,
{
    cursor: CursorMut<'a, K, V>,
    range: Range<K>,
    drained_prev: bool,
}

impl<K, V> Iterator for IntervalDrain<'_, K, V>
where
    K: Clone + Ord,
    V: Interval<K>,
{
    type Item = V;

    fn next(&mut self) -> Option<Self::Item> {
        // There's one previous element that may intersect with the range.
        if !self.drained_prev {
            self.drained_prev = true;
            if let Some((_, v)) = self.cursor.peek_prev() {
                if v.range().end > self.range.start {
                    return Some(self.cursor.remove_prev().unwrap().1);
                }
            }
        }

        // Find all intersected elements following it.
        if let Some((_, v)) = self.cursor.peek_next() {
            if v.range().start >= self.range.end {
                return None;
            }
            return Some(self.cursor.remove_next().unwrap().1);
        }

        None
    }
}

#[cfg(ktest)]
mod tests {
    use alloc::{vec, vec::Vec};
    use core::ops::Range;

    use ostd::prelude::ktest;

    use super::*;

    #[derive(Clone, Debug, PartialEq)]
    struct TestInterval {
        range: Range<i32>,
    }

    impl Interval<i32> for TestInterval {
        fn range(&self) -> Range<i32> {
            self.range.clone()
        }
    }

    #[ktest]
    fn test_insert_and_find_one() {
        let mut set = IntervalSet::new();
        let interval = TestInterval { range: 10..20 };
        set.insert(interval.clone());

        assert_eq!(set.find_one(&15), Some(&interval));
        assert_eq!(set.find_one(&25), None);
    }

    #[ktest]
    fn test_remove() {
        let mut set = IntervalSet::new();
        let interval = TestInterval { range: 10..20 };
        set.insert(interval.clone());

        assert_eq!(set.remove(&10), Some(interval));
        assert_eq!(set.remove(&10), None);
    }

    #[ktest]
    fn test_iter() {
        let mut set = IntervalSet::new();
        let interval1 = TestInterval { range: 10..20 };
        let interval2 = TestInterval { range: 30..40 };
        set.insert(interval1.clone());
        set.insert(interval2.clone());

        let intervals: Vec<&TestInterval> = set.iter().collect();
        assert_eq!(intervals, vec![&interval1, &interval2]);
    }

    #[ktest]
    fn test_find() {
        let mut set = IntervalSet::new();
        let interval1 = TestInterval { range: 10..20 };
        let interval2 = TestInterval { range: 30..40 };
        let interval3 = TestInterval { range: 40..50 };
        let interval4 = TestInterval { range: 80..90 };
        set.insert(interval1.clone());
        set.insert(interval2.clone());
        set.insert(interval3.clone());
        set.insert(interval4.clone());

        let found: Vec<&TestInterval> = set.find(&(35..50)).collect();
        assert_eq!(found, vec![&interval2, &interval3]);
    }

    #[ktest]
    fn test_take_one() {
        let mut set = IntervalSet::new();
        let interval1 = TestInterval { range: 10..20 };
        let interval2 = TestInterval { range: 20..30 };
        set.insert(interval1.clone());
        set.insert(interval2.clone());

        assert_eq!(set.take_one(&15), Some(interval1));
        assert_eq!(set.take_one(&15), None);
    }

    #[ktest]
    fn test_take() {
        let mut set = IntervalSet::new();
        let interval1 = TestInterval { range: 10..20 };
        let interval2 = TestInterval { range: 30..40 };
        let interval3 = TestInterval { range: 45..50 };
        let interval4 = TestInterval { range: 60..70 };
        set.insert(interval1.clone());
        set.insert(interval2.clone());
        set.insert(interval3.clone());
        set.insert(interval4.clone());

        let taken: Vec<TestInterval> = set.take(&(35..45)).collect();
        assert_eq!(taken, vec![interval2]);
    }

    #[ktest]
    fn test_clear() {
        let mut set = IntervalSet::new();
        let interval1 = TestInterval { range: 10..20 };
        let interval2 = TestInterval { range: 20..30 };
        set.insert(interval1);
        set.insert(interval2);

        set.clear();
        assert!(set.iter().next().is_none());
    }
}
