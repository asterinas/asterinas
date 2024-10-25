// SPDX-License-Identifier: MPL-2.0

//! This module contains the implementation of the CPU set and atomic CPU set.

use core::sync::atomic::{AtomicU64, Ordering};

use smallvec::SmallVec;
use static_assertions::const_assert_eq;

use super::{num_cpus, CpuId};

/// A subset of all CPUs in the system.
#[derive(Clone, Debug, Default)]
pub struct CpuSet {
    // A bitset representing the CPUs in the system.
    bits: SmallVec<[InnerPart; NR_PARTS_NO_ALLOC]>,
}

type InnerPart = u64;

const BITS_PER_PART: usize = core::mem::size_of::<InnerPart>() * 8;
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

    /// Adds all CPUs to the set.
    pub fn add_all(&mut self) {
        self.bits.fill(!0);
        self.clear_nonexistent_cpu_bits();
    }

    /// Removes all CPUs from the set.
    pub fn clear(&mut self) {
        self.bits.fill(0);
    }

    /// Iterates over the CPUs in the set.
    ///
    /// The order of the iteration is guaranteed to be in ascending order.
    pub fn iter(&self) -> impl Iterator<Item = CpuId> + '_ {
        self.bits.iter().enumerate().flat_map(|(part_idx, &part)| {
            (0..BITS_PER_PART).filter_map(move |bit_idx| {
                if (part & (1 << bit_idx)) != 0 {
                    let id = part_idx * BITS_PER_PART + bit_idx;
                    Some(CpuId(id as u32))
                } else {
                    None
                }
            })
        })
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
const_assert_eq!(core::mem::size_of::<AtomicInnerPart>() * 8, BITS_PER_PART);

impl AtomicCpuSet {
    /// Creates a new `AtomicCpuSet` with an initial value.
    pub fn new(value: CpuSet) -> Self {
        let bits = value.bits.into_iter().map(AtomicU64::new).collect();
        Self { bits }
    }

    /// Loads the value of the set.
    ///
    /// This operation can only be done in the [`Ordering::Relaxed`] memory
    /// order. It cannot be used to synchronize anything between CPUs.
    pub fn load(&self) -> CpuSet {
        let bits = self
            .bits
            .iter()
            .map(|part| part.load(Ordering::Relaxed))
            .collect();
        CpuSet { bits }
    }

    /// Stores a new value to the set.
    ///
    /// This operation can only be done in the [`Ordering::Relaxed`] memory
    /// order. It cannot be used to synchronize anything between CPUs.
    pub fn store(&self, value: &CpuSet) {
        for (part, new_part) in self.bits.iter().zip(value.bits.iter()) {
            part.store(*new_part, Ordering::Relaxed);
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
        let set_cpus = set.iter().collect::<Vec<_>>();
        assert!(set_cpus.is_empty());
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

            let loaded = atomic_set.load();
            for cpu_id in loaded.iter() {
                if cpu_id.as_usize() % 3 == 0 {
                    assert!(loaded.contains(cpu_id));
                } else {
                    assert!(!loaded.contains(cpu_id));
                }
            }

            atomic_set.store(&CpuSet::with_capacity_val(test_num_cpus, 0));

            for cpu_id in test_all_iter() {
                assert!(!atomic_set.contains(cpu_id, Ordering::Relaxed));
                atomic_set.add(cpu_id, Ordering::Relaxed);
            }
        }
    }
}
