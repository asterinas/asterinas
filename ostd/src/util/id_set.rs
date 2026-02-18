// SPDX-License-Identifier: MPL-2.0

//! A fixed-size set of unique IDs.
//!
//! This module introduces two abstract data collection types.
//! The first one is [`IdSet<I>`],
//! a set that contains at most `I::cardinality()` items,
//! each of which represents a unique ID of type `I`.
//! The second one is [`AtomicIdSet<I>`],
//! the atomic version of `IdSet<I>`.
//!
//! The type parameter `I` implements the [`Id`] trait,
//! which abstracts any ID type whose instances
//! can be 1:1 mapped to the integers from 0 to `Id::cardinality()` (exclusive).
//! The ID type is required to implement `Into<u32>` and `TryFrom<u32>`.
//!
//! One use case of `IdSet<I>` inside OSTD is
//! to implement a set of CPU IDs.
//! [`crate::cpu::CpuSet`] has been simply defined
//! as a type alias for `IdSet<CpuId>`.

use core::{
    fmt::Debug,
    marker::PhantomData,
    ops::{Bound, Range, RangeFrom, RangeFull, RangeTo, RangeToInclusive},
    sync::atomic::{AtomicU64, Ordering},
};

use bitvec::{order::Lsb0, view::BitView};
use smallvec::SmallVec;

use crate::const_assert;

/// A trait to abstract an ID type.
///
/// # Safety
///
/// There must be a 1:1 mapping between this ID type and
/// the integers from 0 to `Self::cardinality()` (exclusive).
/// This implies that the implementation must ensure that
/// if one invokes `Into::<u32>::into()` for an `Id` value,
/// then the returned integer always falls within `0..Id::cardinality()`.
/// Furthermore, the implementation must ensure that
/// for any `id_value` of type `MyId: Id`,
/// the following assertion always succeed
///
/// ```rust
/// assert!(id_value == MyId::new(id_value.into()));
/// ```
///
/// There are also constraints on the implementation of `self::cardinality`.
/// For one thing, the cardinality of the ID type must not change, i.e.,
/// different calls to `self::cardinality` return the same value.
/// In addition, the value of a cardinality must be greater than zero.
pub unsafe trait Id: Copy + Clone + Debug + Eq + Into<u32> + PartialEq {
    /// Creates an ID instance given a raw ID number.
    ///
    /// # Panics
    ///
    /// The given number must be less than `Self::cardinality()`.
    /// Otherwise, this method would panic.
    fn new(raw_id: u32) -> Self {
        assert!(raw_id < Self::cardinality());
        // SAFETY: The raw ID is a valid one.
        unsafe { Self::new_unchecked(raw_id) }
    }

    /// Creates an ID instance given a raw ID number.
    ///
    /// # Safety
    ///
    /// The given number must be less than `Self::cardinality()`.
    unsafe fn new_unchecked(raw_id: u32) -> Self;

    /// The number of unique IDs representable by this type.
    fn cardinality() -> u32;

    /// Returns an [`usize`] from the [`Id`]'s corresponding [`u32`].
    fn as_usize(self) -> usize {
        Into::<u32>::into(self) as usize
    }
}

/// A set of IDs.
///
/// # Examples
///
/// Assume that you have a type named `MyId`,
/// which represents a set of IDs from 0 to 10 (exclusive).
///
/// ```
/// #[derive(Copy, Clone, Debug, Eq, PartialEq)]
/// pub struct MyId(u32);
///
/// // SAFETY: `MyId` maintains the 1:1 mapping invariant for 0..10.
/// unsafe impl Id for MyId {
///     fn new_unchecked(raw_id: u32) -> Self { Self(raw_id) }
///     fn cardinality() -> u32 { 10 } // Fixed cardinality for this example
/// }
/// impl From<MyId> for u32 { fn from(id: MyId) -> u32 { id.0 } }
/// ```
///
/// Now you can use `IdSet<MyId>` as a container for `MyID`s.
///
/// ```
/// let mut my_id_set: IdSet<MyId> = IdSet::new_empty();
///
/// let id0 = MyId::new(0);
/// my_id_set.add(id0);
/// assert!(my_id_set.contains(id0));
/// assert_eq!(my_id_set.count(), 1);
///
/// let id5 = MyId::new(5);
/// my_id_set.add(id5);
/// assert!(my_id_set.contains(id5));
/// assert_eq!(my_id_set.count(), 2);
///
/// my_id_set.remove(id0);
/// assert!(!my_id_set.contains(id0));
/// assert_eq!(my_id_set.count(), 1);
///
/// let full_set = IdSet::<MyId>::new_full();
/// assert_eq!(full_set.count(), 10);
/// assert!(full_set.contains(MyId::new(9)));
/// ```
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IdSet<I> {
    bits: SmallVec<[InnerPart; NR_PARTS_NO_ALLOC]>,
    phantom: PhantomData<I>,
}

type InnerPart = u64;

const BITS_PER_PART: usize = InnerPart::BITS as usize;
const NR_PARTS_NO_ALLOC: usize = 2;

fn part_idx<I: Id>(id: I) -> usize {
    (id.into() as usize) / BITS_PER_PART
}

fn bit_idx<I: Id>(id: I) -> usize {
    (id.into() as usize) % BITS_PER_PART
}

fn parts_for_ids<I: Id>() -> usize {
    (I::cardinality() as usize).div_ceil(BITS_PER_PART)
}

impl<I: Id> IdSet<I> {
    /// Creates a new `IdSet` with all IDs in the system.
    pub fn new_full() -> Self {
        let mut bits = Self::with_bit_pattern(!0);
        Self::clear_invalid_id_bits(&mut bits);
        Self {
            bits,
            phantom: PhantomData,
        }
    }

    /// Creates a new `IdSet` with no IDs in the system.
    pub fn new_empty() -> Self {
        let bits = Self::with_bit_pattern(0);
        Self {
            bits,
            phantom: PhantomData,
        }
    }

    /// Creates a new bitmap with each of its parts filled with the given bits.
    ///
    /// Depending on the bit pattern and the number of IDs,
    /// the resulting bitmap may end with some invalid bits.
    fn with_bit_pattern(part_bits: InnerPart) -> SmallVec<[InnerPart; NR_PARTS_NO_ALLOC]> {
        let num_parts = parts_for_ids::<I>();
        let mut bits = SmallVec::with_capacity(num_parts);
        bits.resize(num_parts, part_bits);
        bits
    }

    fn clear_invalid_id_bits(bits: &mut SmallVec<[InnerPart; NR_PARTS_NO_ALLOC]>) {
        let num_ids = I::cardinality() as usize;
        if !num_ids.is_multiple_of(BITS_PER_PART) {
            let num_parts = parts_for_ids::<I>();
            bits[num_parts - 1] &= (1 << (num_ids % BITS_PER_PART)) - 1;
        }
    }

    /// Adds an ID to the set.
    pub fn add(&mut self, id: I) {
        let part_idx = part_idx(id);
        let bit_idx = bit_idx(id);
        if part_idx >= self.bits.len() {
            self.bits.resize(part_idx + 1, 0);
        }
        self.bits[part_idx] |= 1 << bit_idx;
    }

    /// Removes an ID from the set.
    pub fn remove(&mut self, id: I) {
        let part_idx = part_idx(id);
        let bit_idx = bit_idx(id);
        if part_idx < self.bits.len() {
            self.bits[part_idx] &= !(1 << bit_idx);
        }
    }

    /// Returns true if the set contains the specified ID.
    pub fn contains(&self, id: I) -> bool {
        let part_idx = part_idx(id);
        let bit_idx = bit_idx(id);
        part_idx < self.bits.len() && (self.bits[part_idx] & (1 << bit_idx)) != 0
    }

    /// Returns the number of IDs in the set.
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
        let num_ids = I::cardinality() as usize;
        self.bits.iter().enumerate().all(|(idx, part)| {
            if idx == self.bits.len() - 1 && !num_ids.is_multiple_of(BITS_PER_PART) {
                *part == (1 << (num_ids % BITS_PER_PART)) - 1
            } else {
                *part == !0
            }
        })
    }

    /// Adds all IDs to the set.
    pub fn add_all(&mut self) {
        self.bits.fill(!0);
        Self::clear_invalid_id_bits(&mut self.bits);
    }

    /// Removes all IDs from the set.
    pub fn clear(&mut self) {
        self.bits.fill(0);
    }

    /// Iterates over all IDs in the set.
    ///
    /// The iteration is guaranteed to be in ascending order.
    #[inline]
    pub fn iter(&self) -> impl Iterator<Item = I> + '_ {
        self.iter_in(..)
    }

    /// Iterates over the IDs in the set within the specified range.
    ///
    /// The iteration is guaranteed to be in ascending order.
    /// Only IDs that are both in the set and within the specified range will be returned.
    pub fn iter_in<S: IdSetSlicer<I>>(&self, slicer: S) -> impl Iterator<Item = I> + '_ {
        let (start, end) = slicer.to_range_bounds();

        self.bits.view_bits::<Lsb0>()[start..end]
            .iter_ones()
            .map(move |offset| {
                // SAFETY: `offset` is relative to the slice `[start..end]`,
                // therefore `start + offset` is the absolute index of the bit.
                // Since `offset` only iterates over relative positions of bit 1s, the
                // resulting absolute index must refer to an active bit in `self.bits`.
                unsafe { I::new_unchecked((start + offset) as u32) }
            })
    }
}

/// A trait that unifies all types that slice a portion of [`IdSet`].
pub trait IdSetSlicer<I: Id> {
    /// Converts the index type to inclusive start and exclusive end bounds.
    ///
    /// Returns `(start, end)` where:
    /// - `start`: inclusive lower bound
    /// - `end`: exclusive upper bound
    fn to_range_bounds(self) -> (usize, usize);
}

// In the following implementations of `IdSetSlicer`, the `Id` values are upcast
// from `u32` to `usize`. So adding one is guaranteed to *not* overflow.
impl<I: Id> IdSetSlicer<I> for RangeTo<I> {
    fn to_range_bounds(self) -> (usize, usize) {
        (0, self.end.as_usize())
    }
}
impl<I: Id> IdSetSlicer<I> for RangeFrom<I> {
    fn to_range_bounds(self) -> (usize, usize) {
        (self.start.as_usize(), I::cardinality() as usize)
    }
}
impl<I: Id> IdSetSlicer<I> for Range<I> {
    fn to_range_bounds(self) -> (usize, usize) {
        (self.start.as_usize(), self.end.as_usize())
    }
}
impl<I: Id> IdSetSlicer<I> for RangeFull {
    fn to_range_bounds(self) -> (usize, usize) {
        (0, I::cardinality() as usize)
    }
}
impl<I: Id> IdSetSlicer<I> for RangeToInclusive<I> {
    fn to_range_bounds(self) -> (usize, usize) {
        (0, self.end.as_usize() + 1)
    }
}
impl<I: Id> IdSetSlicer<I> for (Bound<I>, Bound<I>) {
    fn to_range_bounds(self) -> (usize, usize) {
        let (start_bound, end_bound) = self;
        let start = match start_bound {
            Bound::Included(id) => id.as_usize(),
            Bound::Excluded(id) => id.as_usize() + 1,
            Bound::Unbounded => 0,
        };
        let end = match end_bound {
            Bound::Included(id) => id.as_usize() + 1,
            Bound::Excluded(id) => id.as_usize(),
            Bound::Unbounded => I::cardinality() as usize,
        };
        (start, end)
    }
}

impl<I: Id> From<I> for IdSet<I> {
    fn from(id: I) -> Self {
        let mut set = Self::new_empty();
        set.add(id);
        set
    }
}

impl<I: Id> Default for IdSet<I> {
    fn default() -> Self {
        Self::new_empty()
    }
}

/// A set of IDs that may be accessed concurrently.
///
/// `AtomicIdSet` is backed by an array of `AtomicU64`.
/// This allows individual ID bits to be added, removed, or tested atomically.
/// But operations that span multiple `AtomicU64` values are non-atomic.
#[derive(Debug)]
pub struct AtomicIdSet<I> {
    bits: SmallVec<[AtomicInnerPart; NR_PARTS_NO_ALLOC]>,
    phantom: PhantomData<I>,
}

type AtomicInnerPart = AtomicU64;
const_assert!(size_of::<AtomicInnerPart>() == size_of::<InnerPart>());

impl<I: Id> AtomicIdSet<I> {
    /// Creates a new `AtomicIdSet` from an `IdSet`.
    pub fn new(value: IdSet<I>) -> Self {
        let bits = value.bits.into_iter().map(AtomicU64::new).collect();
        Self {
            bits,
            phantom: PhantomData,
        }
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
    pub fn load(&self, ordering: Ordering) -> IdSet<I> {
        let bits = self
            .bits
            .iter()
            .map(|part| match ordering {
                Ordering::Release => part.fetch_or(0, ordering),
                _ => part.load(ordering),
            })
            .collect();
        IdSet {
            bits,
            phantom: PhantomData,
        }
    }

    /// Stores a new value to the set with the given ordering.
    ///
    /// This operation is not atomic. When racing with a [`Self::load`]
    /// operation, that load may return a set that contains a portion of the
    /// new value and a portion of the old value. Load on each specific
    /// word is atomic, and follows the specified ordering.
    pub fn store(&self, value: &IdSet<I>, ordering: Ordering) {
        for (part, new_part) in self.bits.iter().zip(value.bits.iter()) {
            part.store(*new_part, ordering);
        }
    }

    /// Atomically adds an ID with the given ordering.
    pub fn add(&self, id: I, ordering: Ordering) {
        let part_idx = part_idx(id);
        let bit_idx = bit_idx(id);
        if part_idx < self.bits.len() {
            self.bits[part_idx].fetch_or(1 << bit_idx, ordering);
        }
    }

    /// Atomically removes an ID with the given ordering.
    pub fn remove(&self, id: I, ordering: Ordering) {
        let part_idx = part_idx(id);
        let bit_idx = bit_idx(id);
        if part_idx < self.bits.len() {
            self.bits[part_idx].fetch_and(!(1 << bit_idx), ordering);
        }
    }

    /// Atomically checks if the set contains the specified ID.
    pub fn contains(&self, id: I, ordering: Ordering) -> bool {
        let part_idx = part_idx(id);
        let bit_idx = bit_idx(id);
        part_idx < self.bits.len() && (self.bits[part_idx].load(ordering) & (1 << bit_idx)) != 0
    }
}

#[cfg(ktest)]
mod id_set_tests {
    use alloc::vec;

    use super::*;
    use crate::prelude::*;

    /// A mock ID type for testing `IdSet`.
    /// `C` is the cardinality of this ID type.
    #[derive(Copy, Clone, Debug, Eq, PartialEq)]
    struct MockId<const C: u32>(u32);

    unsafe impl<const C: u32> Id for MockId<C> {
        unsafe fn new_unchecked(raw_id: u32) -> Self {
            MockId(raw_id)
        }

        fn cardinality() -> u32 {
            C
        }
    }

    impl<const C: u32> From<MockId<C>> for u32 {
        fn from(id: MockId<C>) -> u32 {
            id.0
        }
    }

    // Test cases for `IdSet` with various cardinalities

    #[ktest]
    fn id_set_empty() {
        type TestId = MockId<10>; // Cardinality 10
        let set: IdSet<TestId> = IdSet::new_empty();
        assert!(set.is_empty());
        assert_eq!(set.count(), 0);
        for i in 0..10 {
            assert!(!set.contains(TestId::new(i)));
        }
    }

    #[ktest]
    fn id_set_full() {
        type TestId = MockId<10>; // Cardinality 10
        let set: IdSet<TestId> = IdSet::new_full();
        assert!(!set.is_empty());
        assert_eq!(set.count(), 10);
        for i in 0..10 {
            assert!(set.contains(TestId::new(i)));
        }
    }

    #[ktest]
    fn id_set_add_remove() {
        type TestId = MockId<64>; // Cardinality 64 (one InnerPart)
        let mut set: IdSet<TestId> = IdSet::new_empty();

        assert!(set.is_empty());

        set.add(TestId::new(0));
        assert!(set.contains(TestId::new(0)));
        assert_eq!(set.count(), 1);
        assert!(!set.is_empty());

        set.add(TestId::new(63));
        assert!(set.contains(TestId::new(63)));
        assert_eq!(set.count(), 2);

        set.add(TestId::new(32));
        assert!(set.contains(TestId::new(32)));
        assert_eq!(set.count(), 3);

        set.remove(TestId::new(0));
        assert!(!set.contains(TestId::new(0)));
        assert_eq!(set.count(), 2);

        set.remove(TestId::new(63));
        assert!(!set.contains(TestId::new(63)));
        assert_eq!(set.count(), 1);

        set.remove(TestId::new(32));
        assert!(!set.contains(TestId::new(32)));
        assert_eq!(set.count(), 0);
        assert!(set.is_empty());

        // Removing a non-existent ID should not panic or change the set
        set.remove(TestId::new(1));
        assert!(set.is_empty());
    }

    #[ktest]
    fn id_set_add_remove_multi_part() {
        type TestId = MockId<128>; // Cardinality 128 (two InnerParts)
        let mut set: IdSet<TestId> = IdSet::new_empty();

        set.add(TestId::new(0));
        set.add(TestId::new(63)); // Last bit of first part
        set.add(TestId::new(64)); // First bit of second part
        set.add(TestId::new(127)); // Last bit of second part

        assert_eq!(set.count(), 4);
        assert!(set.contains(TestId::new(0)));
        assert!(set.contains(TestId::new(63)));
        assert!(set.contains(TestId::new(64)));
        assert!(set.contains(TestId::new(127)));

        set.remove(TestId::new(63));
        assert!(!set.contains(TestId::new(63)));
        assert_eq!(set.count(), 3);

        set.remove(TestId::new(64));
        assert!(!set.contains(TestId::new(64)));
        assert_eq!(set.count(), 2);
    }

    #[ktest]
    fn id_set_add_all_clear() {
        type TestId = MockId<70>; // Cardinality 70 (spans two parts, second part not full)
        let mut set: IdSet<TestId> = IdSet::new_empty();

        set.add_all();
        assert_eq!(set.count(), 70);
        assert!(set.is_full());
        for i in 0..70 {
            assert!(set.contains(TestId::new(i)));
        }

        set.clear();
        assert!(set.is_empty());
        assert_eq!(set.count(), 0);
        for i in 0..70 {
            assert!(!set.contains(TestId::new(i)));
        }
    }

    #[ktest]
    fn id_set_iter() {
        type TestId = MockId<5>; // Cardinality 5
        let mut set: IdSet<TestId> = IdSet::new_empty();

        set.add(TestId::new(2));
        set.add(TestId::new(0));
        set.add(TestId::new(4));

        let collected_ids: Vec<TestId> = set.iter().collect();
        assert_eq!(
            collected_ids,
            vec![TestId::new(0), TestId::new(2), TestId::new(4)]
        );

        set.clear();
        let collected_ids: Vec<TestId> = set.iter().collect();
        assert!(collected_ids.is_empty());
    }

    #[ktest]
    fn id_set_iter_full() {
        type TestId = MockId<3>; // Cardinality 3
        let set: IdSet<TestId> = IdSet::new_full();
        let collected_ids: Vec<TestId> = set.iter().collect();
        assert_eq!(
            collected_ids,
            vec![TestId::new(0), TestId::new(1), TestId::new(2)]
        );
    }

    #[ktest]
    fn id_set_iter_multi_part() {
        type TestId = MockId<100>; // Cardinality 100
        let mut set: IdSet<TestId> = IdSet::new_empty();
        set.add(TestId::new(1));
        set.add(TestId::new(65));
        set.add(TestId::new(99));
        set.add(TestId::new(0));
        set.add(TestId::new(63));

        let collected_ids: Vec<TestId> = set.iter().collect();
        assert_eq!(
            collected_ids,
            vec![
                TestId::new(0),
                TestId::new(1),
                TestId::new(63),
                TestId::new(65),
                TestId::new(99)
            ]
        );
    }

    #[ktest]
    fn id_set_from_id() {
        type TestId = MockId<10>;
        let id = TestId::new(5);
        let set: IdSet<TestId> = id.into();
        assert_eq!(set.count(), 1);
        assert!(set.contains(id));
        assert!(!set.contains(TestId::new(0)));
    }

    #[ktest]
    fn id_set_cardinality_one() {
        type TestId = MockId<1>; // Cardinality 1
        let mut set: IdSet<TestId> = IdSet::new_empty();
        assert!(set.is_empty());
        assert_eq!(set.count(), 0);

        set.add(TestId::new(0));
        assert!(set.contains(TestId::new(0)));
        assert_eq!(set.count(), 1);
        assert!(set.is_full());

        set.remove(TestId::new(0));
        assert!(!set.contains(TestId::new(0)));
        assert_eq!(set.count(), 0);
        assert!(set.is_empty());

        let full_set = IdSet::<TestId>::new_full();
        assert!(full_set.contains(TestId::new(0)));
        assert_eq!(full_set.count(), 1);
    }

    #[ktest]
    fn id_set_exact_part_boundary() {
        type TestId = MockId<64>; // Cardinality exactly one full part
        let mut set: IdSet<TestId> = IdSet::new_empty();

        set.add(TestId::new(0));
        set.add(TestId::new(63));
        assert_eq!(set.count(), 2);

        let full_set = IdSet::<TestId>::new_full();
        assert!(full_set.is_full());
        assert_eq!(full_set.count(), 64);
        for i in 0..64 {
            assert!(full_set.contains(TestId::new(i)));
        }
    }

    #[ktest]
    fn id_set_just_over_part_boundary() {
        type TestId = MockId<65>; // Cardinality just over one part
        let mut set: IdSet<TestId> = IdSet::new_empty();

        set.add(TestId::new(0));
        set.add(TestId::new(63)); // End of first part
        set.add(TestId::new(64)); // Start of second part
        assert_eq!(set.count(), 3);

        let full_set = IdSet::<TestId>::new_full();
        assert!(full_set.is_full());
        assert_eq!(full_set.count(), 65);
        for i in 0..65 {
            assert!(full_set.contains(TestId::new(i)));
        }
    }

    #[ktest]
    fn id_set_is_full_with_less_than_full_last_part() {
        type TestId = MockId<70>; // Cardinality 70 (64 in first part, 6 in second)
        let mut set: IdSet<TestId> = IdSet::new_full();

        assert!(set.is_full());
        assert_eq!(set.count(), 70);

        // Remove one ID from the last part
        set.remove(TestId::new(69));
        assert!(!set.is_full());
        assert_eq!(set.count(), 69);

        // Add it back
        set.add(TestId::new(69));
        assert!(set.is_full());
        assert_eq!(set.count(), 70);
    }

    #[ktest]
    fn id_set_default() {
        type TestId = MockId<10>;
        let set: IdSet<TestId> = Default::default();
        assert!(set.is_empty());
        assert_eq!(set.count(), 0);
    }

    #[ktest]
    fn iter_in_range() {
        type TestId = MockId<7>;
        let mut set: IdSet<TestId> = IdSet::new_empty();
        set.add(TestId::new(0));
        set.add(TestId::new(1));
        set.add(TestId::new(2));
        set.add(TestId::new(5));
        set.add(TestId::new(6));

        let collected_ids: Vec<TestId> = set.iter_in(TestId::new(1)..TestId::new(5)).collect();
        assert_eq!(collected_ids, vec![TestId::new(1), TestId::new(2)],);
    }

    #[ktest]
    fn iter_in_range_to() {
        type TestId = MockId<7>;
        let mut set: IdSet<TestId> = IdSet::new_empty();
        set.add(TestId::new(0));
        set.add(TestId::new(1));
        set.add(TestId::new(2));
        set.add(TestId::new(5));
        set.add(TestId::new(6));

        let collected_ids: Vec<TestId> = set.iter_in(..TestId::new(5)).collect();
        assert_eq!(
            collected_ids,
            vec![TestId::new(0), TestId::new(1), TestId::new(2)],
        );
    }

    #[ktest]
    fn iter_in_range_to_inclusive() {
        type TestId = MockId<7>;
        let mut set: IdSet<TestId> = IdSet::new_empty();
        set.add(TestId::new(0));
        set.add(TestId::new(1));
        set.add(TestId::new(2));
        set.add(TestId::new(5));
        set.add(TestId::new(6));

        let collected_ids: Vec<TestId> = set.iter_in(..=TestId::new(5)).collect();
        assert_eq!(
            collected_ids,
            vec![
                TestId::new(0),
                TestId::new(1),
                TestId::new(2),
                TestId::new(5)
            ],
        );
    }

    #[ktest]
    fn iter_in_range_from() {
        type TestId = MockId<7>;
        let mut set: IdSet<TestId> = IdSet::new_empty();
        set.add(TestId::new(0));
        set.add(TestId::new(1));
        set.add(TestId::new(2));
        set.add(TestId::new(5));
        set.add(TestId::new(6));

        let collected_ids: Vec<TestId> = set.iter_in(TestId::new(2)..).collect();
        assert_eq!(
            collected_ids,
            vec![TestId::new(2), TestId::new(5), TestId::new(6)],
        );
    }

    #[ktest]
    fn iter_in_range_full() {
        type TestId = MockId<7>;
        let mut set: IdSet<TestId> = IdSet::new_empty();
        set.add(TestId::new(0));
        set.add(TestId::new(1));
        set.add(TestId::new(2));
        set.add(TestId::new(5));
        set.add(TestId::new(6));

        let collected_ids: Vec<TestId> = set.iter_in(..).collect();
        assert_eq!(
            collected_ids,
            vec![
                TestId::new(0),
                TestId::new(1),
                TestId::new(2),
                TestId::new(5),
                TestId::new(6)
            ],
        );
    }

    #[ktest]
    fn iter_in_bound_tuple_inclusive_exclusive() {
        type TestId = MockId<7>;
        let mut set: IdSet<TestId> = IdSet::new_empty();
        set.add(TestId::new(0));
        set.add(TestId::new(1));
        set.add(TestId::new(2));
        set.add(TestId::new(5));
        set.add(TestId::new(6));

        let collected_ids: Vec<TestId> = set
            .iter_in((
                Bound::Included(TestId::new(1)),
                Bound::Excluded(TestId::new(5)),
            ))
            .collect();
        assert_eq!(collected_ids, vec![TestId::new(1), TestId::new(2)],);
    }

    #[ktest]
    fn iter_in_bound_tuple_exclusive_inclusive() {
        type TestId = MockId<7>;
        let mut set: IdSet<TestId> = IdSet::new_empty();
        set.add(TestId::new(0));
        set.add(TestId::new(1));
        set.add(TestId::new(2));
        set.add(TestId::new(5));
        set.add(TestId::new(6));

        let collected_ids: Vec<TestId> = set
            .iter_in((
                Bound::Excluded(TestId::new(1)),
                Bound::Included(TestId::new(5)),
            ))
            .collect();
        assert_eq!(collected_ids, vec![TestId::new(2), TestId::new(5)],);
    }

    #[ktest]
    fn iter_in_unbounded_bounds() {
        type TestId = MockId<7>;
        let mut set: IdSet<TestId> = IdSet::new_empty();
        set.add(TestId::new(0));
        set.add(TestId::new(1));
        set.add(TestId::new(2));
        set.add(TestId::new(5));
        set.add(TestId::new(6));

        let collected_ids: Vec<TestId> = set
            .iter_in((Bound::Unbounded::<TestId>, Bound::Unbounded::<TestId>))
            .collect();
        assert_eq!(
            collected_ids,
            vec![
                TestId::new(0),
                TestId::new(1),
                TestId::new(2),
                TestId::new(5),
                TestId::new(6)
            ],
        );
    }

    #[ktest]
    fn iter_in_half_unbounded() {
        type TestId = MockId<7>;
        let mut set: IdSet<TestId> = IdSet::new_empty();
        set.add(TestId::new(0));
        set.add(TestId::new(1));
        set.add(TestId::new(2));
        set.add(TestId::new(5));
        set.add(TestId::new(6));

        let collected_ids: Vec<TestId> = set
            .iter_in((Bound::Included(TestId::new(2)), Bound::Unbounded::<TestId>))
            .collect();
        assert_eq!(
            collected_ids,
            vec![TestId::new(2), TestId::new(5), TestId::new(6)],
        );

        let collected_ids: Vec<TestId> = set
            .iter_in((Bound::Unbounded::<TestId>, Bound::Included(TestId::new(2))))
            .collect();
        assert_eq!(
            collected_ids,
            vec![TestId::new(0), TestId::new(1), TestId::new(2)],
        );
    }

    #[ktest]
    fn iter_in_range_starts_after_last() {
        type TestId = MockId<7>;
        let mut set: IdSet<TestId> = IdSet::new_empty();
        set.add(TestId::new(0));
        set.add(TestId::new(1));
        set.add(TestId::new(2));

        let collected_ids: Vec<TestId> = set.iter_in(TestId::new(3)..).collect();
        assert_eq!(collected_ids, vec![],);
    }

    #[ktest]
    fn iter_in_range_ends_after_last() {
        type TestId = MockId<7>;
        let mut set: IdSet<TestId> = IdSet::new_empty();
        set.add(TestId::new(0));
        set.add(TestId::new(1));
        set.add(TestId::new(2));

        let collected_ids: Vec<TestId> = set.iter_in(..TestId::new(3)).collect();
        assert_eq!(
            collected_ids,
            vec![TestId::new(0), TestId::new(1), TestId::new(2)],
        );
    }

    #[ktest]
    fn iter_in_range_next_part() {
        type TestId = MockId<{ InnerPart::BITS }>;
        let last_id = TestId::new(InnerPart::BITS - 1);

        let mut set: IdSet<TestId> = IdSet::new_empty();
        set.add(last_id);

        let collected_ids: Vec<TestId> = set
            .iter_in((Bound::Excluded(last_id), Bound::Included(last_id)))
            .collect();
        assert_eq!(collected_ids, vec![],);
    }
}
