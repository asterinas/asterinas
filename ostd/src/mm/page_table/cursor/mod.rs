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
    page_size, pte_index, Child, ChildRef, Entry, PageTable, PageTableConfig, PageTableError,
    PageTableGuard, PagingConstsTrait, PagingLevel,
};
use crate::{
    mm::{
        frame::{meta::AnyFrameMeta, Frame},
        page_table::is_valid_range,
        PageProperty, Vaddr,
    },
    task::atomic_mode::InAtomicMode,
};

/// The cursor for traversal over the page table.
///
/// A slot is a PTE at any levels, which correspond to a certain virtual
/// memory range sized by the "page size" of the current level.
///
/// A cursor is able to move to the next slot, to read page properties,
/// and even to jump to a virtual address directly.
#[derive(Debug)]
pub(crate) struct Cursor<'rcu, C: PageTableConfig> {
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
    Mapped { va: Vaddr, item: C::Item },
    /// A sub-tree of a page table that is taken out of the page table.
    ///
    /// The caller is responsible for dropping it after TLB coherence.
    StrayPageTable {
        pt: Frame<dyn AnyFrameMeta>,
        va: Vaddr,
        len: usize,
        num_frames: usize,
    },
}

impl<C: PageTableConfig> PageTableFrag<C> {
    #[cfg(ktest)]
    pub(crate) fn va_range(&self) -> Range<Vaddr> {
        match self {
            PageTableFrag::Mapped { va, item } => {
                let (pa, level, prop) = C::item_into_raw(item.clone());
                // SAFETY: All the arguments match those returned from the previous call
                // to `item_into_raw`, and we are taking ownership of the cloned item.
                drop(unsafe { C::item_from_raw(pa, level, prop) });
                *va..*va + page_size::<C>(level)
            }
            PageTableFrag::StrayPageTable { va, len, .. } => *va..*va + *len,
        }
    }
}

impl<'rcu, C: PageTableConfig> Cursor<'rcu, C> {
    /// Creates a cursor claiming exclusive access over the given range.
    ///
    /// The cursor created will only be able to query or jump within the given
    /// range. Out-of-bound accesses will result in panics or errors as return values,
    /// depending on the access method.
    pub fn new(
        pt: &'rcu PageTable<C>,
        guard: &'rcu dyn InAtomicMode,
        va: &Range<Vaddr>,
    ) -> Result<Self, PageTableError> {
        if !is_valid_range::<C>(va) || va.is_empty() {
            return Err(PageTableError::InvalidVaddrRange(va.start, va.end));
        }
        if va.start % C::BASE_PAGE_SIZE != 0 || va.end % C::BASE_PAGE_SIZE != 0 {
            return Err(PageTableError::UnalignedVaddr);
        }

        const { assert!(C::NR_LEVELS as usize <= MAX_NR_LEVELS) };

        Ok(locking::lock_range(pt, guard, va))
    }

    /// Gets the current virtual address.
    pub fn virt_addr(&self) -> Vaddr {
        self.va
    }

    /// Queries the mapping at the current virtual address.
    ///
    /// If the cursor is pointing to a valid virtual address that is locked,
    /// it will return the virtual address range and the item at that slot.
    pub fn query(&mut self) -> Result<PagesState<C>, PageTableError> {
        if self.va >= self.barrier_va.end {
            return Err(PageTableError::InvalidVaddr(self.va));
        }

        let rcu_guard = self.rcu_guard;

        loop {
            let level = self.level;

            let cur_entry = self.cur_entry();
            let item = match cur_entry.to_ref() {
                ChildRef::PageTable(pt) => {
                    // SAFETY: The `pt` must be locked and no other guards exist.
                    let guard = unsafe { pt.make_guard_unchecked(rcu_guard) };
                    self.push_level(guard);
                    continue;
                }
                ChildRef::None => None,
                ChildRef::Frame(pa, ch_level, prop) => {
                    debug_assert_eq!(ch_level, level);

                    // SAFETY:
                    // This is part of (if `split_huge` happens) a page table item mapped
                    // with a previous call to `C::item_into_raw`, where:
                    //  - The physical address and the paging level match it;
                    //  - The item part is still mapped so we don't take its ownership;
                    //  - The `AVAIL1` flag is preserved by the cursor and the callers of
                    //    the unsafe `protect_next` method.
                    let item = ManuallyDrop::new(unsafe { C::item_from_raw(pa, level, prop) });
                    // TODO: Provide a `PageTableItemRef` to reduce copies.
                    Some((*item).clone())
                }
            };

            return Ok((self.cur_va_range(), item));
        }
    }

    /// Moves the cursor forward to the next mapped virtual address.
    ///
    /// If there is mapped virtual address following the current address within
    /// next `len` bytes, it will return that mapped address. In this case, the
    /// cursor will stop at the mapped address.
    ///
    /// Otherwise, it will return `None`. And the cursor may stop at any
    /// address after `len` bytes.
    ///
    /// # Panics
    ///
    /// Panics if:
    ///  - the length is longer than the remaining range of the cursor;
    ///  - the length is not page-aligned.
    pub fn find_next(&mut self, len: usize) -> Option<Vaddr> {
        self.find_next_impl(len, false, false)
    }

    /// Moves the cursor forward to the next fragment in the range.
    ///
    /// See [`Self::find_next`] for more details. Other than the semantics
    /// provided by [`Self::find_next`], this method also supports finding non-
    /// leaf entries and splitting huge pages if necessary.
    ///
    /// `find_unmap_subtree` specifies whether the cursor should stop at the
    /// highest possible level for unmapping. If `false`, the cursor will only
    /// stop at leaf entries.
    ///
    /// `split_huge` specifies whether the cursor should split huge pages when
    /// it finds a huge page that is mapped over the required range (`len`).
    fn find_next_impl(
        &mut self,
        len: usize,
        find_unmap_subtree: bool,
        split_huge: bool,
    ) -> Option<Vaddr> {
        assert_eq!(len % C::BASE_PAGE_SIZE, 0);
        let end = self.va + len;
        assert!(end <= self.barrier_va.end);
        debug_assert_eq!(end % C::BASE_PAGE_SIZE, 0);

        let rcu_guard = self.rcu_guard;

        while self.va < end {
            let cur_va = self.va;
            let cur_va_range = self.cur_va_range();
            let cur_entry_fits_range = cur_va == cur_va_range.start && cur_va_range.end <= end;

            let mut cur_entry = self.cur_entry();
            match cur_entry.to_ref() {
                ChildRef::PageTable(pt) => {
                    if find_unmap_subtree
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
                        self.move_forward();
                    }
                    continue;
                }
                ChildRef::None => {
                    self.move_forward();
                    continue;
                }
                ChildRef::Frame(_, _, _) => {
                    if cur_entry_fits_range || !split_huge {
                        return Some(cur_va);
                    }

                    let split_child = cur_entry
                        .split_if_mapped_huge(rcu_guard)
                        .expect("The entry must be a huge page");
                    self.push_level(split_child);
                    continue;
                }
            }
        }

        None
    }

    /// Jumps to the given virtual address.
    /// If the target address is out of the range, this method will return `Err`.
    ///
    /// # Panics
    ///
    /// This method panics if the address has bad alignment.
    pub fn jump(&mut self, va: Vaddr) -> Result<(), PageTableError> {
        assert!(va % C::BASE_PAGE_SIZE == 0);
        if !self.barrier_va.contains(&va) {
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
            .expect("Popping a level without a lock");
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

    /// Gets the virtual address range that the current entry covers.
    fn cur_va_range(&self) -> Range<Vaddr> {
        let entry_size = page_size::<C>(self.level);
        let entry_start = self.va.align_down(entry_size);
        entry_start..entry_start + entry_size
    }
}

impl<C: PageTableConfig> Drop for Cursor<'_, C> {
    fn drop(&mut self) {
        locking::unlock_range(self);
    }
}

/// The state of virtual pages represented by a page table.
///
/// This is the return type of the [`Cursor::query`] method.
pub type PagesState<C> = (Range<Vaddr>, Option<<C as PageTableConfig>::Item>);

impl<C: PageTableConfig> Iterator for Cursor<'_, C> {
    type Item = PagesState<C>;

    fn next(&mut self) -> Option<Self::Item> {
        let result = self.query();
        if result.is_ok() {
            self.move_forward();
        }
        result.ok()
    }
}

/// The cursor of a page table that is capable of map, unmap or protect pages.
///
/// It has all the capabilities of a [`Cursor`], which can navigate over the
/// page table corresponding to the address range. A virtual address range
/// in a page table can only be accessed by one cursor, regardless of the
/// mutability of the cursor.
#[derive(Debug)]
pub(crate) struct CursorMut<'rcu, C: PageTableConfig>(Cursor<'rcu, C>);

impl<'rcu, C: PageTableConfig> CursorMut<'rcu, C> {
    /// Creates a cursor claiming exclusive access over the given range.
    ///
    /// The cursor created will only be able to map, query or jump within the given
    /// range. Out-of-bound accesses will result in panics or errors as return values,
    /// depending on the access method.
    pub(super) fn new(
        pt: &'rcu PageTable<C>,
        guard: &'rcu dyn InAtomicMode,
        va: &Range<Vaddr>,
    ) -> Result<Self, PageTableError> {
        Cursor::new(pt, guard, va).map(|inner| Self(inner))
    }

    /// Moves the cursor forward to the next mapped virtual address.
    ///
    /// This is the same as [`Cursor::find_next`].
    pub fn find_next(&mut self, len: usize) -> Option<Vaddr> {
        self.0.find_next(len)
    }

    /// Jumps to the given virtual address.
    ///
    /// This is the same as [`Cursor::jump`].
    ///
    /// # Panics
    ///
    /// This method panics if the address is out of the range where the cursor is required to operate,
    /// or has bad alignment.
    pub fn jump(&mut self, va: Vaddr) -> Result<(), PageTableError> {
        self.0.jump(va)
    }

    /// Gets the current virtual address.
    pub fn virt_addr(&self) -> Vaddr {
        self.0.virt_addr()
    }

    /// Queries the mapping at the current virtual address.
    ///
    /// If the cursor is pointing to a valid virtual address that is locked,
    /// it will return the virtual address range and the item at that slot.
    pub fn query(&mut self) -> Result<PagesState<C>, PageTableError> {
        self.0.query()
    }

    /// Maps the item starting from the current address to a physical address range.
    ///
    /// If the current address has already mapped pages, it will do a re-map,
    /// taking out the old physical address and replacing it with the new one.
    /// This function will return [`Err`] with a [`PageTableFrag`], the not
    /// mapped item. The caller should drop it after TLB coherence.
    ///
    /// If there is no mapped pages in the specified virtual address range,
    /// the function will return [`None`].
    ///
    /// # Panics
    ///
    /// This function will panic if
    ///  - the virtual address range to be mapped is out of the locked range;
    ///  - the current virtual address is not aligned to the page size of the
    ///    item to be mapped;
    ///
    /// # Safety
    ///
    /// The caller should ensure that
    ///  - the range being mapped does not affect kernel's memory safety;
    ///  - the physical address to be mapped is valid and safe to use;
    pub unsafe fn map(&mut self, item: C::Item) -> Result<(), PageTableFrag<C>> {
        assert!(self.0.va < self.0.barrier_va.end);
        let (pa, level, prop) = C::item_into_raw(item);
        assert!(level <= C::HIGHEST_TRANSLATION_LEVEL);
        let size = page_size::<C>(level);
        assert_eq!(self.0.va % size, 0);
        let end = self.0.va + size;
        assert!(end <= self.0.barrier_va.end);

        let rcu_guard = self.0.rcu_guard;

        // Adjust ourselves to the level of the item.
        while self.0.level != level {
            if self.0.level < level {
                self.0.pop_level();
                continue;
            }
            // We are at a higher level, go down.
            let mut cur_entry = self.0.cur_entry();
            match cur_entry.to_ref() {
                ChildRef::PageTable(pt) => {
                    // SAFETY: The `pt` must be locked and no other guards exist.
                    let pt_guard = unsafe { pt.make_guard_unchecked(rcu_guard) };
                    self.0.push_level(pt_guard);
                }
                ChildRef::None => {
                    let child_guard = cur_entry.alloc_if_none(rcu_guard).unwrap();
                    self.0.push_level(child_guard);
                }
                ChildRef::Frame(_, _, _) => {
                    let split_child = cur_entry.split_if_mapped_huge(rcu_guard).unwrap();
                    self.0.push_level(split_child);
                }
            }
        }

        let frag = self.replace_cur_entry(Child::Frame(pa, level, prop));

        self.0.move_forward();

        if let Some(frag) = frag {
            Err(frag)
        } else {
            Ok(())
        }
    }

    /// Finds and removes the first page table fragment in the following range.
    ///
    /// The range to be found in is the current virtual address with the
    /// provided length.
    ///
    /// The function stops and yields the fragment if it has actually removed a
    /// fragment, no matter if the following pages are also required to be
    /// unmapped. The returned virtual address is the virtual page that existed
    /// before the removal but having just been unmapped.
    ///
    /// It also makes the cursor moves forward to the next page after the
    /// removed one, when an actual page is removed. If no mapped pages exist
    /// in the following range, the cursor will stop at the end of the range
    /// and return [`None`].
    ///
    /// The caller should handle TLB coherence if necessary, using the returned
    /// virtual address range.
    ///
    /// # Safety
    ///
    /// The caller should ensure that the range being unmapped does not affect
    /// kernel's memory safety.
    ///
    /// # Panics
    ///
    /// Panics if:
    ///  - the length is longer than the remaining range of the cursor;
    ///  - the length is not page-aligned.
    pub unsafe fn take_next(&mut self, len: usize) -> Option<PageTableFrag<C>> {
        self.0.find_next_impl(len, true, true)?;

        let frag = self.replace_cur_entry(Child::None);

        self.0.move_forward();

        frag
    }

    /// Applies the operation to the next slot of mapping within the range.
    ///
    /// The range to be found in is the current virtual address with the
    /// provided length.
    ///
    /// The function stops and yields the actually protected range if it has
    /// actually protected a page, no matter if the following pages are also
    /// required to be protected.
    ///
    /// It also makes the cursor moves forward to the next page after the
    /// protected one. If no mapped pages exist in the following range, the
    /// cursor will stop at the end of the range and return [`None`].
    ///
    /// # Safety
    ///
    /// The caller should ensure that:
    ///  - the range being protected with the operation does not affect
    ///    kernel's memory safety;
    ///  - the privileged flag `AVAIL1` should not be altered, since this flag
    ///    is reserved for all page tables.
    ///
    /// # Panics
    ///
    /// Panics if:
    ///  - the length is longer than the remaining range of the cursor;
    ///  - the length is not page-aligned.
    pub unsafe fn protect_next(
        &mut self,
        len: usize,
        op: &mut impl FnMut(&mut PageProperty),
    ) -> Option<Range<Vaddr>> {
        self.0.find_next_impl(len, false, true)?;

        self.0.cur_entry().protect(op);

        let protected_va = self.0.cur_va_range();

        self.0.move_forward();

        Some(protected_va)
    }

    fn replace_cur_entry(&mut self, new_child: Child<C>) -> Option<PageTableFrag<C>> {
        let rcu_guard = self.0.rcu_guard;

        let va = self.0.va;
        let level = self.0.level;

        let old = self.0.cur_entry().replace(new_child);
        match old {
            Child::None => None,
            Child::Frame(pa, ch_level, prop) => {
                debug_assert_eq!(ch_level, level);

                // SAFETY:
                // This is part of (if `split_huge` happens) a page table item mapped
                // with a previous call to `C::item_into_raw`, where:
                //  - The physical address and the paging level match it;
                //  - The item part is now unmapped so we can take its ownership;
                //  - The `AVAIL1` flag is preserved by the cursor and the callers of
                //    the unsafe `protect_next` method.
                let item = unsafe { C::item_from_raw(pa, level, prop) };
                Some(PageTableFrag::Mapped { va, item })
            }
            Child::PageTable(pt) => {
                debug_assert_eq!(pt.level(), level - 1);

                if !C::TOP_LEVEL_CAN_UNMAP && level == C::NR_LEVELS {
                    let _ = ManuallyDrop::new(pt); // leak it to make shared PTs stay `'static`.
                    panic!("Unmapping shared kernel page table nodes");
                }

                // SAFETY: We must have locked this node.
                let locked_pt = unsafe { pt.borrow().make_guard_unchecked(rcu_guard) };
                // SAFETY:
                //  - We checked that we are not unmapping shared kernel page table nodes.
                //  - We must have locked the entire sub-tree since the range is locked.
                let num_frames =
                    unsafe { locking::dfs_mark_stray_and_unlock(rcu_guard, locked_pt) };

                Some(PageTableFrag::StrayPageTable {
                    pt: (*pt).clone().into(),
                    va,
                    len: page_size::<C>(self.0.level),
                    num_frames,
                })
            }
        }
    }
}
