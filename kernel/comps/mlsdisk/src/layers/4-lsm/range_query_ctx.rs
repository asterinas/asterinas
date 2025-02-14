// SPDX-License-Identifier: MPL-2.0

// Context for range query.
use core::ops::RangeInclusive;

use super::{RecordKey, RecordValue};
use crate::{prelude::*, util::BitMap};

/// Context for a range query request.
/// It tracks the completing process of each slot within the range.
/// A "slot" indicates one specific key-value pair of the query.
#[derive(Debug)]
pub struct RangeQueryCtx<K, V> {
    start: K,
    num_values: usize,
    complete_table: BitMap,
    min_uncompleted: usize,
    res: Vec<(K, V)>,
}

impl<K: RecordKey<K>, V: RecordValue> RangeQueryCtx<K, V> {
    /// Create a new context with the given start key,
    /// and the number of values for query.
    pub fn new(start: K, num_values: usize) -> Self {
        Self {
            start,
            num_values,
            complete_table: BitMap::repeat(false, num_values),
            min_uncompleted: 0,
            res: Vec::with_capacity(num_values),
        }
    }

    /// Gets the uncompleted range within the whole, returns `None`
    /// if all slots are already completed.
    pub fn range_uncompleted(&self) -> Option<RangeInclusive<K>> {
        if self.is_completed() {
            return None;
        }
        debug_assert!(self.min_uncompleted < self.num_values);

        let first_uncompleted = self.start + self.min_uncompleted;
        let last_uncompleted = self.start + self.complete_table.last_zero()?;
        Some(first_uncompleted..=last_uncompleted)
    }

    /// Whether the uncompleted range contains the target key.
    pub fn contains_uncompleted(&self, key: &K) -> bool {
        let nth = *key - self.start;
        nth < self.num_values && !self.complete_table[nth]
    }

    /// Whether the range query context is completed, means
    /// all slots are filled with the corresponding values.
    pub fn is_completed(&self) -> bool {
        self.min_uncompleted == self.num_values
    }

    /// Complete one slot within the range, with the specific
    /// key and the queried value.
    pub fn complete(&mut self, key: K, value: V) {
        let nth = key - self.start;
        if self.complete_table[nth] {
            return;
        }

        self.res.push((key, value));
        self.complete_table.set(nth, true);
        self.update_min_uncompleted(nth);
    }

    /// Mark the specific slot as completed.
    pub fn mark_completed(&mut self, key: K) {
        let nth = key - self.start;

        self.complete_table.set(nth, true);
        self.update_min_uncompleted(nth);
    }

    /// Turn the context into final results.
    pub fn into_results(self) -> Vec<(K, V)> {
        debug_assert!(self.is_completed());
        self.res
    }

    fn update_min_uncompleted(&mut self, completed_nth: usize) {
        if self.min_uncompleted == completed_nth {
            if let Some(next_uncompleted) = self.complete_table.first_zero(completed_nth) {
                self.min_uncompleted = next_uncompleted;
            } else {
                // Indicate all slots are completed
                self.min_uncompleted = self.num_values;
            }
        }
    }
}
