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

use core::{any::TypeId, marker::PhantomData, ops::Range};

use align_ext::AlignExt;

use super::{
    page_size, pte_index, Child, Entry, KernelMode, MapTrackingStatus, PageTable,
    PageTableEntryTrait, PageTableError, PageTableLock, PageTableMode, PagingConstsTrait,
    PagingLevel, UserMode,
};
use crate::{
    mm::{
        frame::{meta::AnyFrameMeta, Frame},
        Paddr, PageProperty, Vaddr,
    },
    task::DisabledPreemptGuard,
};

/// The cursor for traversal over the page table.
///
/// A slot is a PTE at any levels, which correspond to a certain virtual
/// memory range sized by the "page size" of the current level.
///
/// A cursor is able to move to the next slot, to read page properties,
/// and even to jump to a virtual address directly.
#[derive(Debug)]
pub struct Cursor<'a, M: PageTableMode, E: PageTableEntryTrait, C: PagingConstsTrait> {
    /// The current path of the cursor.
    ///
    /// The level 1 page table lock guard is at index 0, and the level N page
    /// table lock guard is at index N - 1.
    path: [Option<PageTableLock<E, C>>; MAX_NR_LEVELS],
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
    /// This also make all the operation in the `Cursor::new` performed in a
    /// RCU read-side critical section.
    #[expect(dead_code)]
    preempt_guard: DisabledPreemptGuard,
    _phantom: PhantomData<&'a PageTable<M, E, C>>,
}

/// The maximum value of `PagingConstsTrait::NR_LEVELS`.
const MAX_NR_LEVELS: usize = 4;

#[derive(Clone, Debug)]
pub enum PageTableItem {
    NotMapped {
        va: Vaddr,
        len: usize,
    },
    Mapped {
        va: Vaddr,
        page: Frame<dyn AnyFrameMeta>,
        prop: PageProperty,
    },
    MappedUntracked {
        va: Vaddr,
        pa: Paddr,
        len: usize,
        prop: PageProperty,
    },
    StrayPageTable {
        pt: Frame<dyn AnyFrameMeta>,
        va: Vaddr,
        len: usize,
    },
}

impl<'a, M: PageTableMode, E: PageTableEntryTrait, C: PagingConstsTrait> Cursor<'a, M, E, C> {
    /// Creates a cursor claiming exclusive access over the given range.
    ///
    /// The cursor created will only be able to query or jump within the given
    /// range. Out-of-bound accesses will result in panics or errors as return values,
    /// depending on the access method.
    pub fn new(pt: &'a PageTable<M, E, C>, va: &Range<Vaddr>) -> Result<Self, PageTableError> {
        if !M::covers(va) || va.is_empty() {
            return Err(PageTableError::InvalidVaddrRange(va.start, va.end));
        }
        if va.start % C::BASE_PAGE_SIZE != 0 || va.end % C::BASE_PAGE_SIZE != 0 {
            return Err(PageTableError::UnalignedVaddr);
        }
        const { assert!(C::NR_LEVELS as usize <= MAX_NR_LEVELS) };
        let new_pt_is_tracked = if should_map_as_tracked::<M>(va.start) {
            MapTrackingStatus::Tracked
        } else {
            MapTrackingStatus::Untracked
        };
        Ok(locking::lock_range(pt, va, new_pt_is_tracked))
    }

    /// Gets the information of the current slot.
    pub fn query(&mut self) -> Result<PageTableItem, PageTableError> {
        if self.va >= self.barrier_va.end {
            return Err(PageTableError::InvalidVaddr(self.va));
        }

        loop {
            let level = self.level;
            let va = self.va;

            match self.cur_entry().to_ref() {
                Child::PageTableRef(pt) => {
                    // SAFETY: `pt` points to a PT that is attached to a node
                    // in the locked sub-tree, so that it is locked and alive.
                    self.push_level(unsafe { PageTableLock::<E, C>::from_raw_paddr(pt) });
                    continue;
                }
                Child::PageTable(_) => {
                    unreachable!();
                }
                Child::None => {
                    return Ok(PageTableItem::NotMapped {
                        va,
                        len: page_size::<C>(level),
                    });
                }
                Child::Frame(page, prop) => {
                    return Ok(PageTableItem::Mapped { va, page, prop });
                }
                Child::Untracked(pa, plevel, prop) => {
                    debug_assert_eq!(plevel, level);
                    return Ok(PageTableItem::MappedUntracked {
                        va,
                        pa,
                        len: page_size::<C>(level),
                        prop,
                    });
                }
            }
        }
    }

    /// Traverses forward in the current level to the next PTE.
    ///
    /// If reached the end of a page table node, it leads itself up to the next page of the parent
    /// page if possible.
    pub(in crate::mm) fn move_forward(&mut self) {
        let page_size = page_size::<C>(self.level);
        let next_va = self.va.align_down(page_size) + page_size;
        while self.level < self.guard_level && pte_index::<C>(next_va, self.level) == 0 {
            self.pop_level();
        }
        self.va = next_va;
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

    pub fn virt_addr(&self) -> Vaddr {
        self.va
    }

    /// Goes up a level.
    fn pop_level(&mut self) {
        let Some(taken) = self.path[self.level as usize - 1].take() else {
            panic!("Popping a level without a lock");
        };
        let _taken = taken.into_raw_paddr();
        self.level += 1;
    }

    /// Goes down a level to a child page table.
    fn push_level(&mut self, child_pt: PageTableLock<E, C>) {
        self.level -= 1;
        debug_assert_eq!(self.level, child_pt.level());

        let old = self.path[self.level as usize - 1].replace(child_pt);
        debug_assert!(old.is_none());
    }

    fn cur_entry(&mut self) -> Entry<'_, E, C> {
        let node = self.path[self.level as usize - 1].as_mut().unwrap();
        node.entry(pte_index::<C>(self.va, self.level))
    }
}

impl<M: PageTableMode, E: PageTableEntryTrait, C: PagingConstsTrait> Drop for Cursor<'_, M, E, C> {
    fn drop(&mut self) {
        locking::unlock_range(self);
    }
}

impl<M: PageTableMode, E: PageTableEntryTrait, C: PagingConstsTrait> Iterator
    for Cursor<'_, M, E, C>
{
    type Item = PageTableItem;

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
pub struct CursorMut<'a, M: PageTableMode, E: PageTableEntryTrait, C: PagingConstsTrait>(
    Cursor<'a, M, E, C>,
);

impl<'a, M: PageTableMode, E: PageTableEntryTrait, C: PagingConstsTrait> CursorMut<'a, M, E, C> {
    /// Creates a cursor claiming exclusive access over the given range.
    ///
    /// The cursor created will only be able to map, query or jump within the given
    /// range. Out-of-bound accesses will result in panics or errors as return values,
    /// depending on the access method.
    pub(super) fn new(
        pt: &'a PageTable<M, E, C>,
        va: &Range<Vaddr>,
    ) -> Result<Self, PageTableError> {
        Cursor::new(pt, va).map(|inner| Self(inner))
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

    /// Gets the information of the current slot.
    pub fn query(&mut self) -> Result<PageTableItem, PageTableError> {
        self.0.query()
    }

    /// Maps the range starting from the current address to a [`Frame<dyn AnyFrameMeta>`].
    ///
    /// It returns the previously mapped [`Frame<dyn AnyFrameMeta>`] if that exists.
    ///
    /// # Panics
    ///
    /// This function will panic if
    ///  - the virtual address range to be mapped is out of the range;
    ///  - the alignment of the page is not satisfied by the virtual address;
    ///  - it is already mapped to a huge page while the caller wants to map a smaller one.
    ///
    /// # Safety
    ///
    /// The caller should ensure that the virtual range being mapped does
    /// not affect kernel's memory safety.
    pub unsafe fn map(
        &mut self,
        frame: Frame<dyn AnyFrameMeta>,
        prop: PageProperty,
    ) -> Option<Frame<dyn AnyFrameMeta>> {
        let end = self.0.va + frame.size();
        assert!(end <= self.0.barrier_va.end);

        // Go down if not applicable.
        while self.0.level > frame.map_level()
            || self.0.va % page_size::<C>(self.0.level) != 0
            || self.0.va + page_size::<C>(self.0.level) > end
        {
            debug_assert!(should_map_as_tracked::<M>(self.0.va));
            let cur_level = self.0.level;
            let cur_entry = self.0.cur_entry();
            match cur_entry.to_ref() {
                Child::PageTableRef(pt) => {
                    // SAFETY: `pt` points to a PT that is attached to a node
                    // in the locked sub-tree, so that it is locked and alive.
                    self.0
                        .push_level(unsafe { PageTableLock::<E, C>::from_raw_paddr(pt) });
                }
                Child::PageTable(_) => {
                    unreachable!();
                }
                Child::None => {
                    let pt =
                        PageTableLock::<E, C>::alloc(cur_level - 1, MapTrackingStatus::Tracked);
                    let _ = cur_entry.replace(Child::PageTable(pt.clone_raw()));
                    self.0.push_level(pt);
                }
                Child::Frame(_, _) => {
                    panic!("Mapping a smaller frame in an already mapped huge page");
                }
                Child::Untracked(_, _, _) => {
                    panic!("Mapping a tracked page in an untracked range");
                }
            }
            continue;
        }
        debug_assert_eq!(self.0.level, frame.map_level());

        // Map the current page.
        let old = self.0.cur_entry().replace(Child::Frame(frame, prop));
        self.0.move_forward();

        match old {
            Child::Frame(old_page, _) => Some(old_page),
            Child::None => None,
            Child::PageTable(_) => {
                todo!("Dropping page table nodes while mapping requires TLB flush")
            }
            Child::Untracked(_, _, _) => panic!("Mapping a tracked page in an untracked range"),
            Child::PageTableRef(_) => unreachable!(),
        }
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
    /// In practice it is not suggested to use this method for safety and conciseness.
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
    pub unsafe fn map_pa(&mut self, pa: &Range<Paddr>, prop: PageProperty) {
        let end = self.0.va + pa.len();
        let mut pa = pa.start;
        assert!(end <= self.0.barrier_va.end);

        while self.0.va < end {
            // We ensure not mapping in reserved kernel shared tables or releasing it.
            // Although it may be an invariant for all architectures and will be optimized
            // out by the compiler since `C::NR_LEVELS - 1 > C::HIGHEST_TRANSLATION_LEVEL`.
            let is_kernel_shared_node =
                TypeId::of::<M>() == TypeId::of::<KernelMode>() && self.0.level >= C::NR_LEVELS - 1;
            if self.0.level > C::HIGHEST_TRANSLATION_LEVEL
                || is_kernel_shared_node
                || self.0.va % page_size::<C>(self.0.level) != 0
                || self.0.va + page_size::<C>(self.0.level) > end
                || pa % page_size::<C>(self.0.level) != 0
            {
                let cur_level = self.0.level;
                let cur_entry = self.0.cur_entry();
                match cur_entry.to_ref() {
                    Child::PageTableRef(pt) => {
                        // SAFETY: `pt` points to a PT that is attached to a node
                        // in the locked sub-tree, so that it is locked and alive.
                        self.0
                            .push_level(unsafe { PageTableLock::<E, C>::from_raw_paddr(pt) });
                    }
                    Child::PageTable(_) => {
                        unreachable!();
                    }
                    Child::None => {
                        let pt = PageTableLock::<E, C>::alloc(
                            cur_level - 1,
                            MapTrackingStatus::Untracked,
                        );
                        let _ = cur_entry.replace(Child::PageTable(pt.clone_raw()));
                        self.0.push_level(pt);
                    }
                    Child::Frame(_, _) => {
                        panic!("Mapping a smaller page in an already mapped huge page");
                    }
                    Child::Untracked(_, _, _) => {
                        let split_child = cur_entry.split_if_untracked_huge().unwrap();
                        self.0.push_level(split_child);
                    }
                }
                continue;
            }

            // Map the current page.
            debug_assert!(!should_map_as_tracked::<M>(self.0.va));
            let level = self.0.level;
            let _ = self
                .0
                .cur_entry()
                .replace(Child::Untracked(pa, level, prop));

            // Move forward.
            pa += page_size::<C>(level);
            self.0.move_forward();
        }
    }

    /// Find and remove the first page in the cursor's following range.
    ///
    /// The range to be found in is the current virtual address with the
    /// provided length.
    ///
    /// The function stops and yields the page if it has actually removed a
    /// page, no matter if the following pages are also required to be unmapped.
    /// The returned page is the virtual page that existed before the removal
    /// but having just been unmapped.
    ///
    /// It also makes the cursor moves forward to the next page after the
    /// removed one, when an actual page is removed. If no mapped pages exist
    /// in the following range, the cursor will stop at the end of the range
    /// and return [`PageTableItem::NotMapped`].
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
    pub unsafe fn take_next(&mut self, len: usize) -> PageTableItem {
        let start = self.0.va;
        assert!(len % page_size::<C>(1) == 0);
        let end = start + len;
        assert!(end <= self.0.barrier_va.end);

        while self.0.va < end {
            let cur_va = self.0.va;
            let cur_level = self.0.level;
            let cur_entry = self.0.cur_entry();

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
                    Child::PageTableRef(pt) => {
                        // SAFETY: `pt` points to a PT that is attached to a node
                        // in the locked sub-tree, so that it is locked and alive.
                        let pt = unsafe { PageTableLock::<E, C>::from_raw_paddr(pt) };
                        // If there's no mapped PTEs in the next level, we can
                        // skip to save time.
                        if pt.nr_children() != 0 {
                            self.0.push_level(pt);
                        } else {
                            let _ = pt.into_raw_paddr();
                            if self.0.va + page_size::<C>(self.0.level) > end {
                                self.0.va = end;
                                break;
                            }
                            self.0.move_forward();
                        }
                    }
                    Child::PageTable(_) => {
                        unreachable!();
                    }
                    Child::None => {
                        unreachable!("Already checked");
                    }
                    Child::Frame(_, _) => {
                        panic!("Removing part of a huge page");
                    }
                    Child::Untracked(_, _, _) => {
                        let split_child = cur_entry.split_if_untracked_huge().unwrap();
                        self.0.push_level(split_child);
                    }
                }
                continue;
            }

            // Unmap the current page and return it.
            let old = cur_entry.replace(Child::None);

            self.0.move_forward();

            return match old {
                Child::Frame(page, prop) => PageTableItem::Mapped {
                    va: self.0.va,
                    page,
                    prop,
                },
                Child::Untracked(pa, level, prop) => {
                    debug_assert_eq!(level, self.0.level);
                    PageTableItem::MappedUntracked {
                        va: self.0.va,
                        pa,
                        len: page_size::<C>(level),
                        prop,
                    }
                }
                Child::PageTable(pt) => {
                    let paddr = pt.into_raw();
                    // SAFETY: We must have locked this node.
                    let locked_pt = unsafe { PageTableLock::<E, C>::from_raw_paddr(paddr) };
                    assert!(
                        !(TypeId::of::<M>() == TypeId::of::<KernelMode>()
                            && self.0.level == C::NR_LEVELS),
                        "Unmapping shared kernel page table nodes"
                    );
                    // SAFETY:
                    //  - We checked that we are not unmapping shared kernel page table nodes.
                    //  - We must have locked the entire sub-tree since the range is locked.
                    let unlocked_pt = unsafe { locking::dfs_mark_astray(locked_pt) };
                    // See `locking.rs` for why we need this.
                    let drop_after_grace = unlocked_pt.clone();
                    crate::sync::after_grace_period(|| {
                        drop(drop_after_grace);
                    });
                    PageTableItem::StrayPageTable {
                        pt: unlocked_pt.into(),
                        va: self.0.va,
                        len: page_size::<C>(self.0.level),
                    }
                }
                Child::None | Child::PageTableRef(_) => unreachable!(),
            };
        }

        // If the loop exits, we did not find any mapped pages in the range.
        PageTableItem::NotMapped { va: start, len }
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

        while self.0.va < end {
            let cur_va = self.0.va;
            let cur_level = self.0.level;
            let mut cur_entry = self.0.cur_entry();

            // Skip if it is already absent.
            if cur_entry.is_none() {
                self.0.move_forward();
                continue;
            }

            // Go down if it's not a last entry.
            if cur_entry.is_node() {
                let Child::PageTableRef(pt) = cur_entry.to_ref() else {
                    unreachable!("Already checked");
                };
                // SAFETY: `pt` points to a PT that is attached to a node
                // in the locked sub-tree, so that it is locked and alive.
                let pt = unsafe { PageTableLock::<E, C>::from_raw_paddr(pt) };
                // If there's no mapped PTEs in the next level, we can
                // skip to save time.
                if pt.nr_children() != 0 {
                    self.0.push_level(pt);
                } else {
                    pt.into_raw_paddr();
                    self.0.move_forward();
                }
                continue;
            }

            // Go down if the page size is too big and we are protecting part
            // of untracked huge pages.
            if cur_va % page_size::<C>(cur_level) != 0 || cur_va + page_size::<C>(cur_level) > end {
                let split_child = cur_entry
                    .split_if_untracked_huge()
                    .expect("Protecting part of a huge page");
                self.0.push_level(split_child);
                continue;
            }

            // Protect the current page.
            cur_entry.protect(op);

            let protected_va = self.0.va..self.0.va + page_size::<C>(self.0.level);
            self.0.move_forward();

            return Some(protected_va);
        }

        None
    }

    /// Copies the mapping from the given cursor to the current cursor.
    ///
    /// All the mappings in the current cursor's range must be empty. The
    /// function allows the source cursor to operate on the mapping before
    /// the copy happens. So it is equivalent to protect then duplicate.
    /// Only the mapping is copied, the mapped pages are not copied.
    ///
    /// It can only copy tracked mappings since we consider the untracked
    /// mappings not useful to be copied.
    ///
    /// After the operation, both cursors will advance by the specified length.
    ///
    /// # Safety
    ///
    /// The caller should ensure that
    ///  - the range being copied with the operation does not affect kernel's
    ///    memory safety.
    ///  - both of the cursors are in tracked mappings.
    ///
    /// # Panics
    ///
    /// This function will panic if:
    ///  - either one of the range to be copied is out of the range where any
    ///    of the cursor is required to operate;
    ///  - either one of the specified virtual address ranges only covers a
    ///    part of a page.
    ///  - the current cursor's range contains mapped pages.
    pub unsafe fn copy_from(
        &mut self,
        src: &mut Self,
        len: usize,
        op: &mut impl FnMut(&mut PageProperty),
    ) {
        assert!(len % page_size::<C>(1) == 0);
        let this_end = self.0.va + len;
        assert!(this_end <= self.0.barrier_va.end);
        let src_end = src.0.va + len;
        assert!(src_end <= src.0.barrier_va.end);

        while self.0.va < this_end && src.0.va < src_end {
            let src_va = src.0.va;
            let mut src_entry = src.0.cur_entry();

            match src_entry.to_ref() {
                Child::PageTableRef(pt) => {
                    // SAFETY: `pt` points to a PT that is attached to a node
                    // in the locked sub-tree, so that it is locked and alive.
                    let pt = unsafe { PageTableLock::<E, C>::from_raw_paddr(pt) };
                    // If there's no mapped PTEs in the next level, we can
                    // skip to save time.
                    if pt.nr_children() != 0 {
                        src.0.push_level(pt);
                    } else {
                        pt.into_raw_paddr();
                        src.0.move_forward();
                    }
                    continue;
                }
                Child::PageTable(_) => {
                    unreachable!();
                }
                Child::None => {
                    src.0.move_forward();
                    continue;
                }
                Child::Untracked(_, _, _) => {
                    panic!("Copying untracked mappings");
                }
                Child::Frame(page, mut prop) => {
                    let mapped_page_size = page.size();

                    // Do protection.
                    src_entry.protect(op);

                    // Do copy.
                    op(&mut prop);
                    self.jump(src_va).unwrap();
                    let original = self.map(page, prop);
                    assert!(original.is_none());

                    // Only move the source cursor forward since `Self::map` will do it.
                    // This assertion is to ensure that they move by the same length.
                    debug_assert_eq!(mapped_page_size, page_size::<C>(src.0.level));
                    src.0.move_forward();
                }
            }
        }
    }
}

fn should_map_as_tracked<M: PageTableMode>(va: Vaddr) -> bool {
    (TypeId::of::<M>() == TypeId::of::<KernelMode>()
        || TypeId::of::<M>() == TypeId::of::<UserMode>())
        && crate::mm::kspace::should_map_as_tracked(va)
}
