// SPDX-License-Identifier: MPL-2.0

//! This module contains the implementation of the CPU set and atomic CPU set.

use core::sync::atomic::{AtomicU64, Ordering};

use smallvec::SmallVec;

use super::{num_cpus, CpuId};
use crate::const_assert;

/// A subset of all CPUs in the system.
#[derive(Clone, Debug, Default)]
pub struct CpuSet {
    // A bitset representing the CPUs in the system.
    bits: SmallVec<[InnerPart; NR_PARTS_NO_ALLOC]>,
}

type InnerPart = u64;

const BITS_PER_PART: usize = InnerPart::BITS as usize;
const NR_PARTS_NO_ALLOC: usize = 2;

const fn part_idx(cpu_id: CpuId) -> usize {
    cpu_id.as_usize() / BITS_PER_PART
}

const fn bit_idx(cpu_id: CpuId) -> usize {
    cpu_id.as_usize() % BITS_PER_PART
}

const fn parts_for_cpus(num_cpus: usize) -> usize {
    num_cpus.div_ceil(BITS_PER_PART)
}

impl CpuSet {
    /// Creates a new `CpuSet` with all CPUs in the system.
    pub fn new_full() -> Self {
        let mut ret = Self::with_capacity_val(num_cpus(), !0);
        ret.clear_nonexistent_cpu_bits();
        ret
    }

    /// Creates a new `CpuSet` with no CPUs in the system.
    pub fn new_empty() -> Self {
        Self::with_capacity_val(num_cpus(), 0)
    }

    /// Adds a CPU to the set.
    pub fn add(&mut self, cpu_id: CpuId) {
        let part_idx = part_idx(cpu_id);
        let bit_idx = bit_idx(cpu_id);
        if part_idx >= self.bits.len() {
            self.bits.resize(part_idx + 1, 0);
        }
        self.bits[part_idx] |= 1 << bit_idx;
    }

    /// Removes a CPU from the set.
    pub fn remove(&mut self, cpu_id: CpuId) {
        let part_idx = part_idx(cpu_id);
        let bit_idx = bit_idx(cpu_id);
        if part_idx < self.bits.len() {
            self.bits[part_idx] &= !(1 << bit_idx);
        }
    }

    /// Returns true if the set contains the specified CPU.
    pub fn contains(&self, cpu_id: CpuId) -> bool {
        let part_idx = part_idx(cpu_id);
        let bit_idx = bit_idx(cpu_id);
        part_idx < self.bits.len() && (self.bits[part_idx] & (1 << bit_idx)) != 0
    }

    /// Returns the number of CPUs in the set.
    pub fn count(&self) -> usize {
        self.bits
            .iter()
            .map(|part| part.count_ones() as usize)
            .sum()
    }

    /// Returns true if the set is empty.
    pub fn is_empty(&self) -> bool {
        self.bits.iter().all(|part| *part == 0)
    }

    /// Returns true if the set is full.
    pub fn is_full(&self) -> bool {
        let num_cpus = num_cpus();
        self.bits.iter().enumerate().all(|(idx, part)| {
            if idx == self.bits.len() - 1 && num_cpus % BITS_PER_PART != 0 {
                *part == (1 << (num_cpus % BITS_PER_PART)) - 1
            } else {
                *part == !0
            }
        })
    }

    /// Adds all CPUs to the set.
    pub fn add_all(&mut self) {
        self.bits.fill(!0);
        self.clear_nonexistent_cpu_bits();
    }

    /// Removes all CPUs from the set.
    pub fn clear(&mut self) {
        self.bits.fill(0);
    }

    /// Returns an iterator over the CPUs in the set.
    ///
    /// The elements yielded are guaranteed to be in ascending order.
    pub fn iter(&self) -> CpuCycler<'_> {
        CpuCycler::new(self, 0)
    }

    /// Returns an iterator over the CPUs in the set, starting *after* the given
    /// [`CpuId`].
    ///
    /// The iteration order is ascending up to the wrapping point, after which it
    /// continues from the first CPU in the set, in ascending order again.
    ///
    /// If the given [`CpuId`] is in the set, it will be the last element yielded.
    pub fn iter_after(&self, cpu_id: CpuId) -> CpuCycler<'_> {
        CpuCycler::new(self, cpu_id.as_usize() + 1)
    }

    /// Only for internal use. The set cannot contain non-existent CPUs.
    fn with_capacity_val(num_cpus: usize, val: InnerPart) -> Self {
        let num_parts = parts_for_cpus(num_cpus);
        let mut bits = SmallVec::with_capacity(num_parts);
        bits.resize(num_parts, val);
        Self { bits }
    }

    fn clear_nonexistent_cpu_bits(&mut self) {
        let num_cpus = num_cpus();
        if num_cpus % BITS_PER_PART != 0 {
            let num_parts = parts_for_cpus(num_cpus);
            self.bits[num_parts - 1] &= (1 << (num_cpus % BITS_PER_PART)) - 1;
        }
    }
}

/// An iterator that cycles through the CPUs in a [`CpuSet`], starting from a
/// specified position and possibly wrapping around once, at most.
pub struct CpuCycler<'a> {
    cpu_set: &'a CpuSet,
    pos: usize,
    first_found: Option<usize>,
    finished: bool,
}

impl<'a> CpuCycler<'a> {
    fn new(cpu_set: &'a CpuSet, start: usize) -> Self {
        CpuCycler {
            cpu_set,
            pos: start,
            first_found: None,
            finished: false,
        }
    }
}

impl Iterator for CpuCycler<'_> {
    type Item = CpuId;

    fn next(&mut self) -> Option<CpuId> {
        if self.finished {
            return None;
        }

        if self.cpu_set.bits.is_empty() {
            self.finished = true;
            return None;
        }

        loop {
            let part_idx = self.pos / BITS_PER_PART;
            if part_idx >= self.cpu_set.bits.len() {
                // Wrapped around past the highest bit.
                self.pos = 0;
                continue;
            }

            let part = self.cpu_set.bits[part_idx];
            let part_bit_idx = self.pos % BITS_PER_PART;
            let remaining = part >> part_bit_idx;

            if remaining != 0 {
                // Skip trailing zeros for efficiency.
                let nr_zeros = remaining.trailing_zeros() as usize;
                let cpu_idx = part_idx * BITS_PER_PART + part_bit_idx + nr_zeros;

                if let Some(first_found) = self.first_found {
                    if cpu_idx == first_found {
                        // We've cycled back to the first CPU found in the set.
                        self.finished = true;
                        return None;
                    }
                } else {
                    // This is the first CPU found in the set. Cache it.
                    self.first_found = Some(cpu_idx);
                }

                // Advance the position for the next call and return the `CpuId`.
                self.pos = cpu_idx + 1;
                return Some(CpuId(cpu_idx as u32));
            } else {
                // Move to next part.
                self.pos = (part_idx + 1) * BITS_PER_PART;
            }
        }
    }
}

impl From<CpuId> for CpuSet {
    fn from(cpu_id: CpuId) -> Self {
        let mut set = Self::new_empty();
        set.add(cpu_id);
        set
    }
}

/// A subset of all CPUs in the system with atomic operations.
///
/// It provides atomic operations for each CPU in the system. When the
/// operation contains multiple CPUs, the ordering is not guaranteed.
#[derive(Debug)]
pub struct AtomicCpuSet {
    bits: SmallVec<[AtomicInnerPart; NR_PARTS_NO_ALLOC]>,
}

type AtomicInnerPart = AtomicU64;
const_assert!(size_of::<AtomicInnerPart>() * 8 == BITS_PER_PART);

impl AtomicCpuSet {
    /// Creates a new `AtomicCpuSet` with an initial value.
    pub fn new(value: CpuSet) -> Self {
        let bits = value.bits.into_iter().map(AtomicU64::new).collect();
        Self { bits }
    }

    /// Loads the value of the set with the given ordering.
    ///
    /// This operation is not atomic. When racing with a [`Self::store`]
    /// operation, this load may return a set that contains a portion of the
    /// new value and a portion of the old value. Load on each specific
    /// word is atomic, and follows the specified ordering.
    ///
    /// Note that load with [`Ordering::Release`] is a valid operation, which
    /// is different from the normal atomic operations. When coupled with
    /// [`Ordering::Release`], it actually performs `fetch_or(0, Release)`.
    pub fn load(&self, ordering: Ordering) -> CpuSet {
        let bits = self
            .bits
            .iter()
            .map(|part| match ordering {
                Ordering::Release => part.fetch_or(0, ordering),
                _ => part.load(ordering),
            })
            .collect();
        CpuSet { bits }
    }

    /// Stores a new value to the set with the given ordering.
    ///
    /// This operation is not atomic. When racing with a [`Self::load`]
    /// operation, that load may return a set that contains a portion of the
    /// new value and a portion of the old value. Load on each specific
    /// word is atomic, and follows the specified ordering.
    pub fn store(&self, value: &CpuSet, ordering: Ordering) {
        for (part, new_part) in self.bits.iter().zip(value.bits.iter()) {
            part.store(*new_part, ordering);
        }
    }

    /// Atomically adds a CPU with the given ordering.
    pub fn add(&self, cpu_id: CpuId, ordering: Ordering) {
        let part_idx = part_idx(cpu_id);
        let bit_idx = bit_idx(cpu_id);
        if part_idx < self.bits.len() {
            self.bits[part_idx].fetch_or(1 << bit_idx, ordering);
        }
    }

    /// Atomically removes a CPU with the given ordering.
    pub fn remove(&self, cpu_id: CpuId, ordering: Ordering) {
        let part_idx = part_idx(cpu_id);
        let bit_idx = bit_idx(cpu_id);
        if part_idx < self.bits.len() {
            self.bits[part_idx].fetch_and(!(1 << bit_idx), ordering);
        }
    }

    /// Atomically checks if the set contains the specified CPU.
    pub fn contains(&self, cpu_id: CpuId, ordering: Ordering) -> bool {
        let part_idx = part_idx(cpu_id);
        let bit_idx = bit_idx(cpu_id);
        part_idx < self.bits.len() && (self.bits[part_idx].load(ordering) & (1 << bit_idx)) != 0
    }
}

#[cfg(ktest)]
mod test {
    use super::*;
    use crate::{cpu::all_cpus, prelude::*};

    #[ktest]
    fn test_full_cpu_set_iter_is_all() {
        let set = CpuSet::new_full();
        let num_cpus = num_cpus();
        let all_cpus = all_cpus().collect::<Vec<_>>();
        let set_cpus = set.iter().collect::<Vec<_>>();

        assert!(set_cpus.len() == num_cpus);
        assert_eq!(set_cpus, all_cpus);
    }

    #[ktest]
    fn test_full_cpu_set_contains_all() {
        let set = CpuSet::new_full();
        for cpu_id in all_cpus() {
            assert!(set.contains(cpu_id));
        }
    }

    #[ktest]
    fn test_empty_cpu_set_iter_is_empty() {
        let set = CpuSet::new_empty();
        let mut set_cpus = set.iter();
        assert!(set_cpus.next().is_none());
    }

    #[ktest]
    fn test_empty_cpu_set_contains_none() {
        let set = CpuSet::new_empty();
        for cpu_id in all_cpus() {
            assert!(!set.contains(cpu_id));
        }
    }

    #[ktest]
    fn test_atomic_cpu_set_multiple_sizes() {
        for test_num_cpus in [1usize, 3, 12, 64, 96, 99, 128, 256, 288, 1024] {
            let test_all_iter = || (0..test_num_cpus).map(|id| CpuId(id as u32));

            let set = CpuSet::with_capacity_val(test_num_cpus, 0);
            let atomic_set = AtomicCpuSet::new(set);

            for cpu_id in test_all_iter() {
                assert!(!atomic_set.contains(cpu_id, Ordering::Relaxed));
                if cpu_id.as_usize() % 3 == 0 {
                    atomic_set.add(cpu_id, Ordering::Relaxed);
                }
            }

            let loaded = atomic_set.load(Ordering::Relaxed);
            for cpu_id in loaded.iter() {
                if cpu_id.as_usize() % 3 == 0 {
                    assert!(loaded.contains(cpu_id));
                } else {
                    assert!(!loaded.contains(cpu_id));
                }
            }

            atomic_set.store(
                &CpuSet::with_capacity_val(test_num_cpus, 0),
                Ordering::Relaxed,
            );

            for cpu_id in test_all_iter() {
                assert!(!atomic_set.contains(cpu_id, Ordering::Relaxed));
                atomic_set.add(cpu_id, Ordering::Relaxed);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prelude::*;

    fn make_cpu_set(indices: &[usize]) -> CpuSet {
        let mut set = CpuSet::default();
        let num_parts = (indices.iter().max().unwrap_or(&0) / BITS_PER_PART) + 1;
        set.bits.resize(num_parts, 0);
        for &idx in indices {
            let part_idx = idx / BITS_PER_PART;
            let bit_idx = idx % BITS_PER_PART;
            set.bits[part_idx] |= 1 << bit_idx;
        }
        set
    }

    #[test]
    fn iter_single_cpu() {
        let set = make_cpu_set(&[3]);
        let ids: Vec<_> = set.iter().collect();
        assert_eq!(ids, vec![CpuId(3)]);
    }

    #[test]
    fn iter_multiple_cpus() {
        let set = make_cpu_set(&[0, 2, 5, 63, 64, 70]);
        let ids: Vec<_> = set.iter().collect();
        assert_eq!(
            ids,
            vec![
                CpuId(0),
                CpuId(2),
                CpuId(5),
                CpuId(63),
                CpuId(64),
                CpuId(70)
            ]
        );
    }

    #[test]
    fn iter_after_wraps_around() {
        let set = make_cpu_set(&[1, 3, 5, 7]);
        let ids: Vec<_> = set.iter_after(CpuId(3)).collect();
        // Should start after 3, so [5, 7, 1, 3].
        assert_eq!(ids, vec![CpuId(5), CpuId(7), CpuId(1), CpuId(3)]);

        let ids: Vec<_> = set.iter_after(CpuId(4)).collect();
        // Should start after 4, so [5, 7, 1, 3].
        assert_eq!(ids, vec![CpuId(5), CpuId(7), CpuId(1), CpuId(3)]);
    }

    #[test]
    fn iter_after_single_cpu() {
        let set = make_cpu_set(&[42]);
        let ids: Vec<_> = set.iter_after(CpuId(42)).collect();
        // Only one CPU in the set, so iteration should still yield it.
        assert_eq!(ids, vec![CpuId(42)]);
    }

    #[test]
    fn iter_after_beyond_max() {
        let set = make_cpu_set(&[2, 4, 6]);
        let ids: Vec<_> = set.iter_after(CpuId(100)).collect();
        // Start is beyond any present CPUs, so should start at the lowest.
        assert_eq!(ids, vec![CpuId(2), CpuId(4), CpuId(6)]);
    }

    #[test]
    fn iter_and_iter_after_equivalence() {
        let set = make_cpu_set(&(0..10).collect::<Vec<_>>());
        let all: Vec<_> = set.iter().collect();
        let shifted: Vec<_> = set.iter_after(CpuId(4)).collect();
        // Rotated version of all.
        let expected: Vec<_> = (5..10).chain(0..=4).map(|i| CpuId(i as u32)).collect();
        assert_eq!(all, (0..10).map(|i| CpuId(i)).collect::<Vec<_>>());
        assert_eq!(shifted, expected);
    }
}
