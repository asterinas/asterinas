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

use core::{any::TypeId, fmt::Debug, marker::PhantomData, mem::ManuallyDrop, ops::Range};

use align_ext::AlignExt;

use super::{
    page_size, pte_index, Child, ChildRef, Entry, PageTable, PageTableConfig, PageTableError,
    PageTableGuard, PagingConstsTrait, PagingLevel,
};
use crate::{
    mm::{
        frame::{meta::AnyFrameMeta, Frame},
        kspace::KernelPtConfig,
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
    },
}

impl<C: PageTableConfig> PageTableFrag<C> {
    #[cfg(ktest)]
    pub(crate) fn va_range(&self) -> Range<Vaddr> {
        match self {
            PageTableFrag::Mapped { va, item } => {
                let raw = ManuallyDrop::new(C::item_into_raw(item.clone()));
                *va..*va + raw.0.len()
            }
            PageTableFrag::StrayPageTable { va, len, .. } => *va..*va + *len,
        }
    }

    fn from_child(
        rcu_guard: &dyn InAtomicMode,
        child: Child<C>,
        va: Vaddr,
        level: PagingLevel,
    ) -> Option<Self> {
        match child {
            Child::None => None,
            Child::Frame(pa, ch_level, prop) => {
                debug_assert_eq!(ch_level, level);

                let pa_range = pa..pa + page_size::<C>(level);
                // SAFETY: It must be mapped into the page table.
                let item = unsafe { C::item_from_raw(pa_range, prop) };
                Some(PageTableFrag::Mapped { va, item })
            }
            Child::PageTable(pt) => {
                debug_assert_eq!(pt.level(), level - 1);
                // SAFETY: We must have locked this node.
                let locked_pt = unsafe { pt.borrow().make_guard_unchecked(rcu_guard) };
                assert!(
                    !(TypeId::of::<C>() == TypeId::of::<KernelPtConfig>() && level == C::NR_LEVELS),
                    "Unmapping shared kernel page table nodes"
                );
                // SAFETY:
                //  - We checked that we are not unmapping shared kernel page table nodes.
                //  - We must have locked the entire sub-tree since the range is locked.
                unsafe { locking::dfs_mark_stray_and_unlock(rcu_guard, locked_pt) };

                Some(PageTableFrag::StrayPageTable {
                    pt: (*pt).clone().into(),
                    va,
                    len: page_size::<C>(level),
                })
            }
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
            let cur_va = self.va;
            let level = self.level;

            let cur_child = self.cur_entry().to_ref();
            let item = match cur_child {
                ChildRef::PageTable(pt) => {
                    // SAFETY: The `pt` must be locked and no other guards exist.
                    let guard = unsafe { pt.make_guard_unchecked(rcu_guard) };
                    self.push_level(guard);
                    continue;
                }
                ChildRef::None => None,
                ChildRef::Frame(pa, ch_level, prop) => {
                    debug_assert_eq!(ch_level, level);

                    let pa_range = pa..pa + page_size::<C>(level);
                    // SAFETY: It must be mapped into the page table.
                    let item = unsafe { C::item_from_raw(pa_range, prop) };
                    // Clone a copy so that the page table still has one ownership.
                    // TODO: Provide a `PageTableItemRef` to reduce copies.
                    let _ = ManuallyDrop::new(item.clone());
                    Some(item)
                }
            };

            return Ok((cur_va..cur_va + page_size::<C>(level), item));
        }
    }

    /// Moves the cursor forward to the next mapped virtual address.
    ///
    /// If there is mapped virtual address following the current address within
    /// next `len` bytes, it will return that mapped address. In this case,
    /// the cursor will stop at the mapped address.
    ///
    /// Otherwise, it will return `None`. And the cursor may stop at any
    /// address after `len` bytes.
    pub fn find_next(&mut self, len: usize) -> Option<Vaddr> {
        let rcu_guard = self.rcu_guard;

        let end = self.va.saturating_add(len).min(self.barrier_va.end);

        while self.va < end {
            let cur_va = self.va;
            let cur_entry = self.cur_entry();

            match cur_entry.to_ref() {
                ChildRef::PageTable(pt) => {
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
                    return Some(cur_va);
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
            let cur_node_start = self.va & !(page_size::<C>(self.level + 1) - 1);
            let cur_node_end = cur_node_start + page_size::<C>(self.level + 1);
            // If the address is within the current node, we can jump directly.
            if cur_node_start <= va && va < cur_node_end {
                self.va = va;
                return Ok(());
            }

            // There is a corner case that the cursor is depleted, sitting at the start of the
            // next node but the next node is not locked because the parent is not locked.
            if self.va >= self.barrier_va.end && self.level == self.guard_level {
                self.va = va;
                return Ok(());
            }

            debug_assert!(self.level < self.guard_level);
            self.pop_level();
        }
    }

    /// Traverses forward in the current level to the next entry.
    ///
    /// It will move the cursor forward by exactly one page size at the current
    /// level. For example, if the cursor is at level 2, it will move forward
    /// by `page_size::<C>(2)`, which is 2MiB for x86_64.
    ///
    /// If reached the end of the current page table node, it (recursively)
    /// moves itself up to the next page of the parent page.
    fn move_forward(&mut self) {
        let page_size = page_size::<C>(self.level);
        let next_va = self.va.align_down(page_size) + page_size;
        while self.level < self.guard_level && pte_index::<C>(next_va, self.level) == 0 {
            self.pop_level();
        }
        self.va = next_va;
    }

    /// Goes up a level.
    fn pop_level(&mut self) {
        let Some(taken) = self.path[self.level as usize - 1].take() else {
            panic!("Popping a level without a lock");
        };
        let _ = ManuallyDrop::new(taken);

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

    /// Maps the range starting from the current address to a physical address range.
    ///
    /// The function will map as more huge pages as possible, and it will split
    /// the huge pages into smaller pages if necessary. If the input range is
    /// large, the resulting mappings may look like this (if very huge pages
    /// supported):
    ///
    /// ```text
    /// start                                                             end
    ///   |----|----------------|--------------------------------|----|----|
    ///    base      huge                     very huge           base base
    ///    4KiB      2MiB                       1GiB              4KiB  4KiB
    /// ```
    ///
    /// If this function encounters already mapped pages in the specified
    /// virtual address range, it will do a re-map, taking out the old physical
    /// address and replacing it with the new one. This function will return
    /// [`Err`] with a [`PageTableFrag`], the not mapped item, and halt now,
    /// meaning that:
    ///  - all virtual addresses before the end of the re-mapped range are
    ///    mapped successfully;
    ///  - virtual addresses after the end of the re-mapped range are not
    ///    touched;
    ///  - the provided page table item is truncated and returned if there
    ///    are any physical addresses left to be mapped;
    ///  - the cursor will locate after the end of the re-mapped range.
    ///
    /// If there is no mapped pages in the specified virtual address range,
    /// the function will return [`None`].
    ///
    /// # Panics
    ///
    /// This function will panic if
    ///  - the virtual address range to be mapped is out of the range.
    ///
    /// # Safety
    ///
    /// The caller should ensure that
    ///  - the range being mapped does not affect kernel's memory safety;
    ///  - the physical address to be mapped is valid and safe to use;
    ///  - it is allowed to map untracked pages in this virtual address range.
    pub unsafe fn map(&mut self, item: C::Item) -> Result<(), CursorMutMapError<C>> {
        let rcu_guard = self.0.rcu_guard;

        let (pa, prop) = C::item_into_raw(item);
        let start_va = self.0.va;
        let end_va = start_va + pa.len();

        let mut cur_pa = pa.start;
        assert!(end_va <= self.0.barrier_va.end);

        while self.0.va < end_va {
            // We ensure not mapping in reserved kernel shared tables or releasing it.
            // Although it may be an invariant for all architectures and will be optimized
            // out by the compiler since `C::NR_LEVELS - 1 > C::HIGHEST_TRANSLATION_LEVEL`.
            let is_kernel_shared_node = TypeId::of::<C>() == TypeId::of::<KernelPtConfig>()
                && self.0.level >= C::NR_LEVELS - 1;
            if self.0.level > C::HIGHEST_TRANSLATION_LEVEL
                || is_kernel_shared_node
                || self.0.va % page_size::<C>(self.0.level) != 0
                || self.0.va + page_size::<C>(self.0.level) > end_va
                || cur_pa % page_size::<C>(self.0.level) != 0
            {
                let mut cur_entry = self.0.cur_entry();
                match cur_entry.to_ref() {
                    ChildRef::PageTable(pt) => {
                        // SAFETY: The `pt` must be locked and no other guards exist.
                        let guard = unsafe { pt.make_guard_unchecked(rcu_guard) };
                        self.0.push_level(guard);
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
                continue;
            }

            let cur_level = self.0.level;
            let cur_va = self.0.va;

            // Map the current page.
            let old = self
                .0
                .cur_entry()
                .replace(Child::Frame(cur_pa, cur_level, prop));
            let old_item = PageTableFrag::<C>::from_child(rcu_guard, old, cur_va, cur_level);

            // Move forward.
            cur_pa += page_size::<C>(cur_level);
            self.0.move_forward();

            if let Some(frag) = old_item {
                let remain = if cur_pa < pa.end {
                    // SAFETY: The physical address range is valid because it
                    // is truncated from the caller provided original range.
                    Some(unsafe { C::item_from_raw(cur_pa..pa.end, prop) })
                } else {
                    None
                };
                return Err(CursorMutMapError { frag, remain });
            }
        }

        Ok(())
    }

    /// Find and remove the first page table fragment in the following range.
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
    /// This function will panic if the end range covers a part of a huge page
    /// and the next page is that huge page.
    pub unsafe fn take_next(&mut self, len: usize) -> Option<PageTableFrag<C>> {
        let start = self.0.va;
        assert!(len % page_size::<C>(1) == 0);
        let end = start + len;
        assert!(end <= self.0.barrier_va.end);

        let rcu_guard = self.0.rcu_guard;

        while self.0.va < end {
            let cur_va = self.0.va;
            let cur_level = self.0.level;
            let mut cur_entry = self.0.cur_entry();

            // Skip if it is already absent.
            if cur_entry.is_none() {
                if self.0.va + page_size::<C>(self.0.level) > end {
                    self.0.va = end;
                    break;
                }
                self.0.move_forward();
                continue;
            }

            // Go down if not applicable.
            if cur_va % page_size::<C>(cur_level) != 0 || cur_va + page_size::<C>(cur_level) > end {
                let child = cur_entry.to_ref();
                match child {
                    ChildRef::PageTable(pt) => {
                        // SAFETY: The `pt` must be locked and no other guards exist.
                        let pt = unsafe { pt.make_guard_unchecked(rcu_guard) };
                        // If there's no mapped PTEs in the next level, we can
                        // skip to save time.
                        if pt.nr_children() != 0 {
                            self.0.push_level(pt);
                        } else {
                            let _ = ManuallyDrop::new(pt);
                            if self.0.va + page_size::<C>(self.0.level) > end {
                                self.0.va = end;
                                break;
                            }
                            self.0.move_forward();
                        }
                    }
                    ChildRef::None => {
                        unreachable!("Already checked");
                    }
                    ChildRef::Frame(_, _, _) => {
                        let split_child = cur_entry.split_if_mapped_huge(rcu_guard).unwrap();
                        self.0.push_level(split_child);
                    }
                }
                continue;
            }

            // Unmap the current page and return it.
            let old = cur_entry.replace(Child::None);
            let item = PageTableFrag::<C>::from_child(rcu_guard, old, cur_va, cur_level);

            self.0.move_forward();

            return item;
        }

        // If the loop exits, we did not find any mapped pages in the range.
        None
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
    /// The caller should ensure that the range being protected with the
    /// operation does not affect kernel's memory safety.
    ///
    /// # Panics
    ///
    /// This function will panic if:
    ///  - the range to be protected is out of the range where the cursor
    ///    is required to operate;
    ///  - the specified virtual address range only covers a part of a page.
    pub unsafe fn protect_next(
        &mut self,
        len: usize,
        op: &mut impl FnMut(&mut PageProperty),
    ) -> Option<Range<Vaddr>> {
        let end = self.0.va + len;
        assert!(end <= self.0.barrier_va.end);

        let rcu_guard = self.0.rcu_guard;

        let cur_va = self.0.find_next(len)?;
        if cur_va >= end {
            return None;
        }

        let mut cur_level = self.0.level;
        let mut cur_entry = self.0.cur_entry();

        // Go down if the page size is too big and we are protecting part
        // of untracked huge pages.
        while cur_va + page_size::<C>(cur_level) > end {
            let split_child = cur_entry
                .split_if_mapped_huge(rcu_guard)
                .expect("The entry must be a huge page");
            self.0.push_level(split_child);
            cur_level = self.0.level;
            cur_entry = self.0.cur_entry();
        }

        // Protect the current page.
        cur_entry.protect(op);

        let protected_va = self.0.va..self.0.va + page_size::<C>(self.0.level);
        self.0.move_forward();

        Some(protected_va)
    }
}

/// The returned type of the [`CursorMut::map`] method.
#[derive(Debug)]
pub struct CursorMutMapError<C: PageTableConfig> {
    /// The page table fragment that is previously mapped.
    pub frag: PageTableFrag<C>,
    /// The remaining item that is required to be mapped but not mapped.
    pub remain: Option<C::Item>,
}
