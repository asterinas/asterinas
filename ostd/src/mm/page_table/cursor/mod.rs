// SPDX-License-Identifier: MPL-2.0

//! The page table cursor for mapping and querying over the page table.
//!
//! # The page table lock protocol
//!
//! We provide a fine-grained ranged mutual-exclusive lock protocol to allow
//! concurrent accesses to non-overlapping virtual ranges in the page table.
//!
//! [`CursorMut::new`] will lock a range in the virtual space and all the
//! operations on the range with the cursor will be atomic as a transaction.
//!
//! The guarantee of the lock protocol is that, if two cursors' ranges overlap,
//! all of one's operation must be finished before any of the other's
//! operation. The order depends on the scheduling of the threads. If a cursor
//! is ordered after another cursor, it will see all the changes made by the
//! previous cursor.
//!
//! The implementation of the lock protocol resembles two-phase locking (2PL).
//! [`CursorMut::new`] accepts an address range, which indicates the page table
//! entries that may be visited by this cursor. Then, [`CursorMut::new`] finds
//! an intermediate page table (not necessarily the last-level or the top-
//! level) which represents an address range that fully contains the whole
//! specified address range. Then it locks all the nodes in the sub-tree rooted
//! at the intermediate page table node, with a pre-order DFS order. The cursor
//! will only be able to access the page table entries in the locked range.
//! Upon destruction, the cursor will release the locks in the reverse order of
//! acquisition.

mod locking;

use core::{fmt::Debug, marker::PhantomData, mem::ManuallyDrop, ops::Range};

use align_ext::AlignExt;

use super::{
    Entry, PageTable, PageTableConfig, PageTableError, PageTableGuard, PagingConstsTrait,
    PagingLevel, PteState, PteStateRef, page_size, pte_index,
};
use crate::{
    mm::{
        PageProperty, Vaddr,
        page_table::{PageTableNode, is_valid_range},
    },
    sync::RcuDrop,
    task::atomic_mode::InAtomicMode,
};

/// The cursor for traversal over the page table.
///
/// At any time, the cursor points to a page table entry in a certain level of
/// the page table hierarchy. And the entry have a corresponding virtual
/// address range, which covers the current virtual address of the cursor.
///
/// The current virtual address and level must be within the locked range of
/// the cursor.
pub(crate) type Cursor<'rcu, C> = Cursor_<'rcu, C, false>;

/// The cursor of a page table that is capable of map, unmap or protect pages.
///
/// It has all the capabilities of a [`Cursor`], which can navigate over the
/// page table corresponding to the address range. A virtual address range
/// in a page table can only be accessed by one cursor, regardless of the
/// mutability of the cursor.
pub(crate) type CursorMut<'rcu, C> = Cursor_<'rcu, C, true>;

/// The cursor for traversing the page table.
///
/// The `MUTABLE` generic parameter indicates whether the cursor has mutable
/// access to the page table entries. Prefer to use [`CursorMut`] and [`Cursor`]
/// types aliases.
#[derive(Debug)]
pub(crate) struct Cursor_<'rcu, C: PageTableConfig, const MUTABLE: bool = false> {
    /// The current path of the cursor.
    ///
    /// The level 1 page table lock guard is at index 0, and the level N page
    /// table lock guard is at index N - 1.
    path: [Option<PageTableGuard<'rcu, C>>; MAX_NR_LEVELS],
    /// The cursor should be used in a RCU read side critical section.
    rcu_guard: &'rcu dyn InAtomicMode,
    /// The level of the page table that the cursor currently points to.
    level: PagingLevel,
    /// The top-most level that the cursor is allowed to access.
    ///
    /// From `level` to `guard_level`, the nodes are held in `path`.
    guard_level: PagingLevel,
    /// The virtual address that the cursor currently points to.
    va: Vaddr,
    /// The virtual address range that is locked.
    barrier_va: Range<Vaddr>,
    _phantom: PhantomData<&'rcu PageTable<C>>,
}

/// The maximum value of `PagingConstsTrait::NR_LEVELS`.
const MAX_NR_LEVELS: usize = 4;

/// A fragment of a page table that can be taken out of the page table.
#[derive(Debug)]
#[must_use]
pub(crate) enum PageTableFrag<C: PageTableConfig> {
    /// A mapped page table item.
    Mapped { va: Vaddr, item: RcuDrop<C::Item> },
    /// A sub-tree of a page table that is taken out of the page table.
    ///
    /// The caller is responsible for dropping it after TLB coherence.
    StrayPageTable {
        pt: RcuDrop<PageTableNode<C>>,
        va: Vaddr,
        len: usize,
        num_frames: usize,
    },
}

impl<'rcu, C: PageTableConfig, const MUTABLE: bool> Cursor_<'rcu, C, MUTABLE> {
    /// Creates a cursor claiming exclusive access over the given range.
    ///
    /// The cursor created will only be able to query or jump within the given range.
    /// Out-of-bound accesses will result in panics or errors as return values,
    /// depending on the access method.
    pub fn new(
        pt: &'rcu PageTable<C>,
        guard: &'rcu dyn InAtomicMode,
        va: &Range<Vaddr>,
    ) -> Result<Self, PageTableError> {
        if !is_valid_range::<C>(va) || va.is_empty() {
            return Err(PageTableError::InvalidVaddrRange(va.start, va.end));
        }
        if !va.start.is_multiple_of(C::BASE_PAGE_SIZE) || !va.end.is_multiple_of(C::BASE_PAGE_SIZE)
        {
            return Err(PageTableError::UnalignedVaddr);
        }

        const { assert!(C::NR_LEVELS as usize <= MAX_NR_LEVELS) };

        Ok(locking::lock_range(pt, guard, va))
    }

    /// Gets the current virtual address.
    pub fn virt_addr(&self) -> Vaddr {
        self.va
    }

    /// Gets the virtual address range that the current entry covers.
    pub fn cur_va_range(&self) -> Range<Vaddr> {
        let entry_size = page_size::<C>(self.level);
        let entry_start = self.va.align_down(entry_size);
        entry_start..entry_start + entry_size
    }

    /// Gets the current level of the cursor.
    pub fn level(&self) -> PagingLevel {
        self.level
    }

    /// Queries the mapping at the current virtual address.
    pub fn query(&mut self) -> Option<C::ItemRef<'rcu>> {
        debug_assert!(self.barrier_va.contains(&self.va));

        let rcu_guard = self.rcu_guard;

        loop {
            let cur_entry = self.cur_entry();
            let item = match cur_entry.to_ref() {
                PteStateRef::PageTable(pt) => {
                    // SAFETY: The `pt` must be locked and no other guards exist.
                    let guard = unsafe { pt.make_guard_unchecked(rcu_guard) };
                    self.push_level(guard);
                    continue;
                }
                PteStateRef::Absent => None,
                PteStateRef::Mapped(item) => Some(item),
            };

            return item;
        }
    }

    /// Moves the cursor forward to the next mapped virtual address.
    ///
    /// If there is mapped virtual address following the current address within
    /// next `len` bytes, it will return that mapped address. In this case, the
    /// cursor will stop at the mapped address.
    ///
    /// Otherwise, it will return `None`. And the cursor may stop at any
    /// address within `len` bytes.
    ///
    /// # Panics
    ///
    /// Panics if:
    ///  - the length is longer than the remaining range of the cursor;
    ///  - the length is not page-aligned.
    pub fn find_next(&mut self, len: usize) -> Option<Vaddr> {
        self.find_next_impl(len, false)
    }

    /// Moves the cursor forward to the largest possible subtree that contains
    /// mapped pages.
    ///
    /// This is similar to [`Self::find_next`], except that the cursor will
    /// stop at the highest possible level, that the subtree's virtual address
    /// range is fully covered by `len`. This is useful for
    /// [`CursorMut::unmap`].
    ///
    /// # Panics
    ///
    /// Panics if:
    ///  - the length is longer than the remaining range of the cursor;
    ///  - the length is not page-aligned.
    pub fn find_next_unmappable_subtree(&mut self, len: usize) -> Option<Vaddr> {
        self.find_next_impl(len, true)
    }

    fn find_next_impl(&mut self, len: usize, find_subtree: bool) -> Option<Vaddr> {
        assert_eq!(len % C::BASE_PAGE_SIZE, 0);
        let end = self.va + len;
        assert!(end <= self.barrier_va.end);
        debug_assert_eq!(end % C::BASE_PAGE_SIZE, 0);

        let rcu_guard = self.rcu_guard;

        while self.va < end {
            while find_subtree && self.entry_at_level_fits_unmap(self.level + 1, end) {
                self.pop_level();
            }

            let cur_va = self.va;
            let cur_va_range = self.cur_va_range();
            let cur_entry_fits_range = self.entry_at_level_fits_unmap(self.level, end);

            match self.cur_entry().to_ref() {
                PteStateRef::PageTable(pt) => {
                    if find_subtree
                        && cur_entry_fits_range
                        && (C::TOP_LEVEL_CAN_UNMAP || self.level != C::NR_LEVELS)
                    {
                        return Some(cur_va);
                    }

                    // SAFETY: The `pt` must be locked and no other guards exist.
                    let pt_guard = unsafe { pt.make_guard_unchecked(rcu_guard) };
                    // If there's no mapped PTEs in the next level, we can
                    // skip to save time.
                    if pt_guard.nr_children() != 0 {
                        self.push_level(pt_guard);
                    } else {
                        let _ = ManuallyDrop::new(pt_guard);
                        if cur_va_range.end >= end {
                            break;
                        } else {
                            self.move_forward();
                        }
                    }
                    continue;
                }
                PteStateRef::Absent => {
                    if cur_va_range.end >= end {
                        break;
                    } else {
                        self.move_forward();
                    }
                    continue;
                }
                PteStateRef::Mapped(_) => {
                    return Some(cur_va);
                }
            }
        }

        None
    }

    fn entry_at_level_fits_unmap(&self, level: PagingLevel, end: Vaddr) -> bool {
        if (level > self.guard_level) || (level == C::NR_LEVELS && !C::TOP_LEVEL_CAN_UNMAP) {
            return false;
        }
        let entry_size = page_size::<C>(level);
        let entry_start = self.va.align_down(entry_size);
        entry_start == self.va && entry_start + entry_size <= end
    }

    /// Jumps to the given virtual address.
    ///
    /// If the target address is out of the range or if the address is not
    /// base-page-aligned, this method will return `Err`.
    pub fn jump(&mut self, va: Vaddr) -> Result<(), PageTableError> {
        if !va.is_multiple_of(C::BASE_PAGE_SIZE) || !self.barrier_va.contains(&va) {
            return Err(PageTableError::InvalidVaddr(va));
        }

        loop {
            let node_size = page_size::<C>(self.level + 1);
            let node_start = self.va.align_down(node_size);
            // If the address is within the current node, we can jump directly.
            if node_start <= va && va < node_start + node_size {
                self.va = va;
                return Ok(());
            }

            self.pop_level();
        }
    }

    /// Traverses forward to the end of [`Self::cur_va_range`].
    ///
    /// If reached the end of the current page table node, it (recursively)
    /// moves itself up to the next page of the parent page.
    fn move_forward(&mut self) {
        let next_va = self.cur_va_range().end;
        while self.level < self.guard_level && pte_index::<C>(next_va, self.level) == 0 {
            self.pop_level();
        }
        self.va = next_va;
    }

    /// Goes up a level.
    fn pop_level(&mut self) {
        let taken = self.path[self.level as usize - 1]
            .take()
            .expect("popping a level without a lock");
        let _ = ManuallyDrop::new(taken);

        debug_assert!(self.level < self.guard_level);
        self.level += 1;
    }

    /// Goes down a level to a child page table.
    fn push_level(&mut self, child_pt: PageTableGuard<'rcu, C>) {
        self.level -= 1;
        debug_assert_eq!(self.level, child_pt.level());

        let old = self.path[self.level as usize - 1].replace(child_pt);
        debug_assert!(old.is_none());
    }

    fn cur_entry(&mut self) -> Entry<'_, 'rcu, C> {
        let node = self.path[self.level as usize - 1].as_mut().unwrap();
        node.entry(pte_index::<C>(self.va, self.level))
    }
}

impl<C: PageTableConfig> CursorMut<'_, C> {
    /// Adjust to the given level.
    ///
    /// When the specified level page table is not allocated, it will allocate
    /// and go to that page table. If the current virtual address contains a
    /// huge mapping, and the specified level is lower than the mapping, it
    /// will split the huge mapping into smaller mappings.
    ///
    /// # Panics
    ///
    /// Panics if the specified level is invalid.
    pub fn adjust_level(&mut self, to: PagingLevel) {
        assert!(1 <= to && to <= C::NR_LEVELS);

        let rcu_guard = self.rcu_guard;

        while self.level != to {
            if self.level < to {
                self.pop_level();
                continue;
            }
            // We are at a higher level, go down.
            let mut cur_entry = self.cur_entry();
            match cur_entry.to_ref() {
                PteStateRef::PageTable(pt) => {
                    // SAFETY: The `pt` must be locked and no other guards exist.
                    let pt_guard = unsafe { pt.make_guard_unchecked(rcu_guard) };
                    self.push_level(pt_guard);
                }
                PteStateRef::Absent => {
                    let child_guard = cur_entry.alloc_if_none(rcu_guard).unwrap();
                    self.push_level(child_guard);
                }
                PteStateRef::Mapped(_) => {
                    let split_child = cur_entry.split_if_mapped_huge(rcu_guard).unwrap();
                    self.push_level(split_child);
                }
            }
        }
    }

    /// Maps the item starting from the current address to a physical address range.
    ///
    /// The current virtual address should not be mapped.
    ///
    /// # Panics
    ///
    /// This function will panic if
    ///  - the current virtual address is not aligned to the page size of the
    ///    item to be mapped;
    ///  - the end of the current virtual address range exceeds the locked range;
    ///  - the current virtual address range contains mappings.
    ///
    /// # Safety
    ///
    /// The caller should ensure that
    ///  - the range being mapped does not affect kernel's memory safety;
    ///  - the physical address to be mapped is valid and safe to use.
    pub unsafe fn map(&mut self, item: C::Item) {
        debug_assert!(self.va < self.barrier_va.end);

        let (_, level, _) = C::item_raw_info(&item);
        assert!(
            level <= C::HIGHEST_TRANSLATION_LEVEL,
            "cursor level not suitable for mapping"
        );
        let size = page_size::<C>(level);
        assert_eq!(
            self.va % size,
            0,
            "cursor virtual address not aligned for mapping"
        );
        let end = self.va + size;
        assert!(
            end <= self.barrier_va.end,
            "cursor virtual address out-of-bound for mapping"
        );

        self.adjust_level(level);

        if !matches!(self.cur_entry().to_ref(), PteStateRef::Absent) {
            panic!("mapping over an already mapped page");
        }

        let _ = self.replace_cur_entry(PteState::Mapped(RcuDrop::new(item)));
    }

    /// Removes the page table fragment at the current PTE.
    ///
    /// The unmapped virtual address range depends on the current level of the
    /// cursor, and can be queried via [`Self::cur_va_range`]. Adjust the
    /// level via [`Self::adjust_level`] before unmapping to change the
    ///
    /// The caller should handle TLB coherence if necessary, using the returned
    /// virtual address range.
    ///
    /// # Safety
    ///
    /// The caller should ensure that:
    ///  - the range being unmapped does not affect kernel's memory safety.
    ///  - the items mapped in `PageTableFrag` must outlive any TLB entries
    ///    that cache the mappings.
    ///
    /// # Panics
    ///
    /// Panics if the current level is at the top level and the corresponding
    /// [`PageTableConfig::TOP_LEVEL_CAN_UNMAP`] is false.
    pub unsafe fn unmap(&mut self) -> Option<PageTableFrag<C>> {
        if !C::TOP_LEVEL_CAN_UNMAP && self.level == C::NR_LEVELS {
            panic!("Unmapping top-level page table nodes");
        }
        self.replace_cur_entry(PteState::Absent)
    }

    /// Applies the operation to the current PTE.
    ///
    /// The unmapped virtual address range depends on the current level of the
    /// cursor, and can be queried via [`Self::cur_va_range`]. Adjust the
    /// level via [`Self::adjust_level`] before unmapping to change the
    ///
    /// It only modifies the page properties of the current entry state is
    /// [`PteState::Mapped`]. Otherwise, it does nothing.
    ///
    /// # Safety
    ///
    /// The caller should ensure that:
    ///  - the range being protected with the operation does not affect
    ///    kernel's memory safety;
    ///  - the privileged flag `AVAIL1` should not be altered, since this flag
    ///    is reserved for all page tables.
    pub unsafe fn protect(&mut self, op: &mut impl FnMut(&mut PageProperty)) {
        self.cur_entry().protect(op);
    }

    fn replace_cur_entry(&mut self, new_child: PteState<C>) -> Option<PageTableFrag<C>> {
        let rcu_guard = self.rcu_guard;

        let va = self.va;
        let level = self.level;

        let old = self.cur_entry().replace(new_child);
        match old {
            PteState::Absent => None,
            PteState::Mapped(item) => Some(PageTableFrag::Mapped { va, item }),
            PteState::PageTable(pt) => {
                debug_assert_eq!(pt.level(), level - 1);

                if !C::TOP_LEVEL_CAN_UNMAP && level == C::NR_LEVELS {
                    let _ = ManuallyDrop::new(pt); // leak it to make shared PTs stay `'static`.
                    panic!("unmapping shared kernel page table nodes");
                }

                // SAFETY: We must have locked this node.
                let locked_pt = unsafe { pt.borrow().make_guard_unchecked(rcu_guard) };
                // SAFETY:
                //  - We checked that we are not unmapping shared kernel page table nodes.
                //  - We must have locked the entire sub-tree since the range is locked.
                let num_frames =
                    unsafe { locking::dfs_mark_stray_and_unlock(rcu_guard, locked_pt) };

                Some(PageTableFrag::StrayPageTable {
                    pt,
                    va,
                    len: page_size::<C>(self.level),
                    num_frames,
                })
            }
        }
    }
}

impl<C: PageTableConfig, const MUTABLE: bool> Drop for Cursor_<'_, C, MUTABLE> {
    fn drop(&mut self) {
        locking::unlock_range(self);
    }
}
