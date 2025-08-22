// SPDX-License-Identifier: MPL-2.0

//! A data structure that tracks a contiguous range of counters.

// TODO: A segment tree can optimize every operations to `O(log(range size))`.
// We just keep it simple for now, since it should just be used for small
// ranges like DMA and I/O memory.

use alloc::{collections::btree_map::BTreeMap, vec::Vec};
use core::ops::Range;

/// A contiguous range of counters.
pub(crate) struct RangeCounter {
    counters: BTreeMap<usize, usize>,
}

impl RangeCounter {
    /// Creates a new [`RangeCounter`].
    ///
    /// The default value for all counters is zero.
    pub(crate) const fn new() -> Self {
        Self {
            counters: BTreeMap::new(),
        }
    }

    /// Returns the counter at the index.
    #[cfg(ktest)]
    pub(crate) fn get(&self, index: usize) -> usize {
        self.counters.get(&index).cloned().unwrap_or(0)
    }

    /// Adds one new count for all counters in the range.
    ///
    /// Returns ranges that the counter has updated from zero to one.
    ///
    /// # Panics
    ///
    /// Panics if the range has a negative size.
    pub(crate) fn add(&mut self, range: &Range<usize>) -> impl Iterator<Item = Range<usize>> {
        assert!(range.start <= range.end);
        let mut updated_ranges = Vec::new();
        let mut reported_end = range.start;

        for i in range.clone() {
            let counter = self.counters.entry(i).or_insert(0);
            if *counter != 0 {
                if reported_end < i {
                    updated_ranges.push(reported_end..i);
                }
                reported_end = i + 1;
            }
            *counter += 1;
        }
        if reported_end < range.end {
            updated_ranges.push(reported_end..range.end);
        }

        updated_ranges.into_iter()
    }

    /// Removes one count for all counters in the range.
    ///
    /// Returns ranges that the counter has updated from one to zero.
    ///
    /// # Panics
    ///
    /// Panics if
    ///  - the range has a negative size, or
    ///  - the range contains a counter that is already zero.
    pub(crate) fn remove(&mut self, range: &Range<usize>) -> impl Iterator<Item = Range<usize>> {
        assert!(range.start <= range.end);
        let mut updated_ranges = Vec::new();
        let mut reported_end = range.start;

        for i in range.clone() {
            let counter = self.counters.get_mut(&i).expect("Removing a zero counter");
            if *counter > 1 {
                if reported_end < i {
                    updated_ranges.push(reported_end..i);
                }
                reported_end = i + 1;
            }
            *counter -= 1;
            if *counter == 0 {
                self.counters.remove(&i);
            }
        }
        if reported_end < range.end {
            updated_ranges.push(reported_end..range.end);
        }

        updated_ranges.into_iter()
    }
}

#[cfg(ktest)]
mod test {
    use alloc::vec;

    use super::*;
    use crate::prelude::*;

    /// A macro to check counter values for multiple ranges.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// check_counter_values!(counter, [15..20, 1], [20..30, 2], [30..35, 1])
    /// ```
    macro_rules! check_counter_values {
        ($counter:expr, $([$range:expr, $expected:expr]),* $(,)?) => {
            $(
                for i in $range {
                    assert_eq!($counter.get(i), $expected,
                        "Counter at index {} should be {}, but got {}",
                        i, $expected, $counter.get(i));
                }
            )*
        };
    }

    #[ktest]
    fn add_remove_range() {
        let mut counter = RangeCounter::new();
        let range = 0..5;

        assert_eq!(counter.add(&range).collect::<Vec<_>>(), vec![range.clone()]);
        check_counter_values!(counter, [range.clone(), 1]);

        assert_eq!(
            counter.remove(&range).collect::<Vec<_>>(),
            vec![range.clone()]
        );
        check_counter_values!(counter, [range, 0]);
    }

    #[ktest]
    fn add_remove_overlapping_beginning() {
        let mut counter = RangeCounter::new();
        let range1 = 10..15;
        let range2 = 3..13;

        assert_eq!(
            counter.add(&range1).collect::<Vec<_>>(),
            vec![range1.clone()]
        );
        assert_eq!(counter.add(&range2).collect::<Vec<_>>(), vec![3..10]);

        check_counter_values!(counter, [3..10, 1], [10..13, 2], [13..15, 1]);

        assert_eq!(counter.remove(&range2).collect::<Vec<_>>(), vec![3..10]);

        check_counter_values!(counter, [3..10, 0], [10..13, 1], [13..15, 1]);
    }

    #[ktest]
    fn add_remove_overlapping_end() {
        let mut counter = RangeCounter::new();
        let range1 = 10..15;
        let range2 = 12..18;

        assert_eq!(
            counter.add(&range1).collect::<Vec<_>>(),
            vec![range1.clone()]
        );
        assert_eq!(counter.add(&range2).collect::<Vec<_>>(), vec![15..18]);

        check_counter_values!(counter, [10..12, 1], [12..15, 2], [15..18, 1]);

        assert_eq!(counter.remove(&range2).collect::<Vec<_>>(), vec![15..18]);

        check_counter_values!(counter, [10..12, 1], [12..15, 1], [15..18, 0]);
    }

    #[ktest]
    fn add_remove_covering() {
        let mut counter = RangeCounter::new();
        let range1 = 20..30;
        let range2 = 15..35;

        assert_eq!(
            counter.add(&range1).collect::<Vec<_>>(),
            vec![range1.clone()]
        );
        assert_eq!(
            counter.add(&range2).collect::<Vec<_>>(),
            vec![15..20, 30..35]
        );

        check_counter_values!(counter, [15..20, 1], [20..30, 2], [30..35, 1]);

        assert_eq!(
            counter.remove(&range2).collect::<Vec<_>>(),
            vec![15..20, 30..35]
        );

        check_counter_values!(counter, [15..20, 0], [20..30, 1], [30..35, 0]);
    }

    #[ktest]
    fn add_remove_partial_overlap() {
        let mut counter = RangeCounter::new();
        let range1 = 5..15;
        let range2 = 10..20;
        let remove_range = 8..12;

        assert_eq!(
            counter.add(&range1).collect::<Vec<_>>(),
            vec![range1.clone()]
        );
        assert_eq!(counter.add(&range2).collect::<Vec<_>>(), vec![15..20]);

        check_counter_values!(counter, [5..10, 1], [10..15, 2], [15..20, 1]);

        assert_eq!(
            counter.remove(&remove_range).collect::<Vec<_>>(),
            vec![8..10]
        );

        check_counter_values!(
            counter,
            [5..8, 1],
            [8..10, 0],
            [10..12, 1],
            [12..15, 2],
            [15..20, 1]
        );
    }
}
