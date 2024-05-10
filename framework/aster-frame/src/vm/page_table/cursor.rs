// SPDX-License-Identifier: MPL-2.0

//! The page table cursor for mapping and querying over the page table.
//!
//! ## The page table lock protocol
//!
//! We provide a fine-grained lock protocol to allow concurrent accesses to
//! the page table. The protocol is originally proposed by Ruihan Li
//! <lrh2000@pku.edu.cn>.
//!
//! [`CursorMut::new`] accepts an address range, which indicates the page table
//! entries that may be visited by this cursor.
//!
//! Then, [`CursorMut::new`] finds an intermediate page table (not necessarily
//! the last-level or the top-level) which represents an address range that contains
//! the whole specified address range. It requires all locks from the root page
//! table to the intermediate page table, but then unlocks all locks excluding the
//! one for the intermediate page table. CursorMut then maintains the lock
//! guards from one for the intermediate page table to the leaf that the cursor is
//! currently manipulating.
//!
//! For example, if we're going to map the address range shown below:
//!
//! ```plain
//! Top-level page table node             A
//!                                      /
//!                                     B
//!                                    / \
//! Last-level page table nodes       C   D
//! Last-level PTEs               ---**...**---
//!                                  \__ __/
//!                                     V
//!                  Address range that we're going to map
//! ```
//!
//! When calling [`CursorMut::new`], it will:
//!  1. `lock(A)`, `lock(B)`, `unlock(A)`;
//!  2. `guards = [ locked(B) ]`.
//!
//! When calling [`CursorMut::map`], it will:
//!  1. `lock(C)`, `guards = [ locked(B), locked(C) ]`;
//!  2. Map some pages in `C`;
//!  3. `unlock(C)`, `lock_guard = [ locked(B) ]`;
//!  4. `lock(D)`, `lock_guard = [ locked(B), locked(D) ]`;
//!  5. Map some pages in D;
//!  6. `unlock(D)`, `lock_guard = [ locked(B) ]`;
//!
//! If all the mappings in `B` are cancelled when cursor finished it's traversal,
//! and `B` need to be recycled, a page walk from the root page table to `B` is
//! required. The cursor unlock all locks, then lock all the way down to `B`, then
//! check if `B` is empty, and finally recycle all the resources on the way back.

use alloc::sync::Arc;
use core::{any::TypeId, ops::Range};

use align_ext::AlignExt;

use super::{
    nr_ptes_per_node, page_size, pte_index, Child, KernelMode, PageTable, PageTableEntryTrait,
    PageTableError, PageTableFrame, PageTableMode, PagingConstsTrait,
};
use crate::{
    sync::{ArcSpinLockGuard, SpinLock},
    vm::{Paddr, PageProperty, Vaddr, VmFrame},
};

/// The cursor for traversal over the page table.
///
/// Efficient methods are provided to move the cursor forward by a slot,
/// doing mapping, unmaping, or querying for the traversed slot. Also you
/// can jump forward or backward by re-walking without releasing the lock.
///
/// A slot is a PTE at any levels, which correspond to a certain virtual
/// memory range sized by the "page size" of the current level.
///
/// Doing mapping is somewhat like a depth-first search on a tree, except
/// that we modify the tree while traversing it. We use a guard stack to
/// simulate the recursion, and adpot a page table locking protocol to
/// provide concurrency.
pub(crate) struct CursorMut<'a, M: PageTableMode, E: PageTableEntryTrait, C: PagingConstsTrait>
where
    [(); nr_ptes_per_node::<C>()]:,
    [(); C::NR_LEVELS]:,
{
    pt: &'a PageTable<M, E, C>,
    guards: [Option<ArcSpinLockGuard<PageTableFrame<E, C>>>; C::NR_LEVELS],
    level: usize,             // current level
    guard_level: usize,       // from guard_level to level, the locks are held
    va: Vaddr,                // current virtual address
    barrier_va: Range<Vaddr>, // virtual address range that is locked
}

impl<'a, M: PageTableMode, E: PageTableEntryTrait, C: PagingConstsTrait> CursorMut<'a, M, E, C>
where
    [(); nr_ptes_per_node::<C>()]:,
    [(); C::NR_LEVELS]:,
{
    /// Create a cursor exclusively owning the locks for the given range.
    ///
    /// The cursor created will only be able to map, query or jump within the
    /// given range.
    pub(crate) fn new(
        pt: &'a PageTable<M, E, C>,
        va: &Range<Vaddr>,
    ) -> Result<Self, PageTableError> {
        if !M::covers(va) {
            return Err(PageTableError::InvalidVaddrRange(va.start, va.end));
        }
        if va.start % C::BASE_PAGE_SIZE != 0 || va.end % C::BASE_PAGE_SIZE != 0 {
            return Err(PageTableError::UnalignedVaddr);
        }
        // Create a guard array that only hold the root node lock.
        let guards = core::array::from_fn(|i| {
            if i == 0 {
                Some(pt.root_frame.lock_arc())
            } else {
                None
            }
        });
        let mut cursor = Self {
            pt,
            guards,
            level: C::NR_LEVELS,
            guard_level: C::NR_LEVELS,
            va: va.start,
            barrier_va: va.clone(),
        };
        // Go down and get proper locks. The cursor should hold a lock of a
        // page table node containing the virtual address range.
        //
        // While going down, previous guards of too-high levels will be released.
        loop {
            let level_too_high = {
                let start_idx = pte_index::<C>(va.start, cursor.level);
                let end_idx = pte_index::<C>(va.end - 1, cursor.level);
                start_idx == end_idx
            };
            if !level_too_high || !cursor.cur_child().is_pt() {
                break;
            }
            cursor.level_down(None);
            cursor.guards[C::NR_LEVELS - cursor.level - 1] = None;
            cursor.guard_level -= 1;
        }
        Ok(cursor)
    }

    /// Jump to the given virtual address.
    ///
    /// It panics if the address is out of the range where the cursor is required to operate,
    /// or has bad alignment.
    pub(crate) fn jump(&mut self, va: Vaddr) {
        assert!(self.barrier_va.contains(&va));
        assert!(va % C::BASE_PAGE_SIZE == 0);
        loop {
            let cur_node_start = self.va & !(page_size::<C>(self.level + 1) - 1);
            let cur_node_end = cur_node_start + page_size::<C>(self.level + 1);
            // If the address is within the current node, we can jump directly.
            if cur_node_start <= va && va < cur_node_end {
                self.va = va;
                return;
            }
            // There is a corner case that the cursor is depleted, sitting at the start of the
            // next node but the next node is not locked because the parent is not locked.
            if self.va >= self.barrier_va.end && self.level == self.guard_level {
                self.va = va;
                return;
            }
            debug_assert!(self.level < self.guard_level);
            self.level_up();
        }
    }

    /// Map the range starting from the current address to a `VmFrame`.
    ///
    /// # Panic
    ///
    /// This function will panic if
    ///  - the virtual address range to be mapped is out of the range;
    ///  - it is already mapped to a huge page while the caller wants to map a smaller one.
    ///
    /// # Safety
    ///
    /// The caller should ensure that the virtual range being mapped does
    /// not affect kernel's memory safety.
    pub(crate) unsafe fn map(&mut self, frame: VmFrame, prop: PageProperty) {
        let end = self.va + C::BASE_PAGE_SIZE;
        assert!(end <= self.barrier_va.end);
        // Go down if not applicable.
        while self.level > C::HIGHEST_TRANSLATION_LEVEL
            || self.va % page_size::<C>(self.level) != 0
            || self.va + page_size::<C>(self.level) > end
        {
            self.level_down(Some(prop));
            continue;
        }
        // Map the current page.
        let idx = self.cur_idx();
        let level = self.level;
        self.cur_node_mut()
            .set_child(idx, Child::Frame(frame), Some(prop), level > 1);
        self.move_forward();
    }

    /// Map the range starting from the current address to a physical address range.
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
    /// # Safety
    ///
    /// The caller should ensure that
    ///  - the range being mapped does not affect kernel's memory safety;
    ///  - the physical address to be mapped is valid and safe to use.
    pub(crate) unsafe fn map_pa(&mut self, pa: &Range<Paddr>, prop: PageProperty) {
        let end = self.va + pa.len();
        let mut pa = pa.start;
        assert!(end <= self.barrier_va.end);
        while self.va < end {
            // We ensure not mapping in reserved kernel shared tables or releasing it.
            // Although it may be an invariant for all architectures and will be optimized
            // out by the compiler since `C::NR_LEVELS - 1 > C::HIGHEST_TRANSLATION_LEVEL`.
            let is_kernel_shared_node =
                TypeId::of::<M>() == TypeId::of::<KernelMode>() && self.level >= C::NR_LEVELS - 1;
            if self.level > C::HIGHEST_TRANSLATION_LEVEL
                || is_kernel_shared_node
                || self.va % page_size::<C>(self.level) != 0
                || self.va + page_size::<C>(self.level) > end
                || pa % page_size::<C>(self.level) != 0
            {
                self.level_down(Some(prop));
                continue;
            }
            // Map the current page.
            let idx = self.cur_idx();
            let level = self.level;
            self.cur_node_mut()
                .set_child(idx, Child::Untracked(pa), Some(prop), level > 1);
            pa += page_size::<C>(level);
            self.move_forward();
        }
    }

    /// Unmap the range starting from the current address with the given length of virtual address.
    ///
    /// # Safety
    ///
    /// The caller should ensure that the range being unmapped does not affect kernel's memory safety.
    ///
    /// # Panic
    ///
    /// This function will panic if:
    ///  - the range to be unmapped is out of the range where the cursor is required to operate;
    ///  - the range covers only a part of a page.
    pub(crate) unsafe fn unmap(&mut self, len: usize) {
        let end = self.va + len;
        assert!(end <= self.barrier_va.end);
        assert!(end % C::BASE_PAGE_SIZE == 0);
        while self.va < end {
            // Skip if it is already invalid.
            if self.cur_child().is_none() {
                if self.va + page_size::<C>(self.level) > end {
                    break;
                }
                self.move_forward();
                continue;
            }

            // We check among the conditions that may lead to a level down.
            // We ensure not unmapping in reserved kernel shared tables or releasing it.
            let is_kernel_shared_node =
                TypeId::of::<M>() == TypeId::of::<KernelMode>() && self.level >= C::NR_LEVELS - 1;
            if is_kernel_shared_node
                || self.va % page_size::<C>(self.level) != 0
                || self.va + page_size::<C>(self.level) > end
            {
                self.level_down(Some(PageProperty::new_absent()));
                continue;
            }

            // Unmap the current page.
            let idx = self.cur_idx();
            self.cur_node_mut().set_child(idx, Child::None, None, false);
            self.move_forward();
        }
    }

    /// Apply the given operation to all the mappings within the range.
    ///
    /// The funtction will return an error if it is not allowed to protect an invalid range and
    /// it does so, or if the range to be protected only covers a part of a page.
    ///
    /// # Safety
    ///
    /// The caller should ensure that the range being protected does not affect kernel's memory safety.
    ///
    /// # Panic
    ///
    /// This function will panic if:
    ///  - the range to be protected is out of the range where the cursor is required to operate.
    pub(crate) unsafe fn protect(
        &mut self,
        len: usize,
        mut op: impl FnMut(&mut PageProperty),
        allow_protect_invalid: bool,
    ) -> Result<(), PageTableError> {
        let end = self.va + len;
        assert!(end <= self.barrier_va.end);
        while self.va < end {
            if self.cur_child().is_none() {
                if !allow_protect_invalid {
                    return Err(PageTableError::ProtectingInvalid);
                }
                self.move_forward();
                continue;
            }
            // Go down if it's not a last node.
            if self.cur_child().is_pt() {
                self.level_down(None);
                continue;
            }
            let vaddr_not_fit = self.va % page_size::<C>(self.level) != 0
                || self.va + page_size::<C>(self.level) > end;
            let mut pte_prop = self.read_cur_pte_prop();
            op(&mut pte_prop);
            // Go down if the page size is too big and we are protecting part
            // of untyped huge pages.
            if self.cur_child().is_untyped() && vaddr_not_fit {
                self.level_down(Some(pte_prop));
                continue;
            } else if vaddr_not_fit {
                return Err(PageTableError::ProtectingPartial);
            }
            let idx = self.cur_idx();
            let level = self.level;
            self.cur_node_mut().protect(idx, pte_prop, level);
            self.move_forward();
        }
        Ok(())
    }

    /// Get the information of the current slot and move to the next slot.
    pub(crate) fn query(&mut self) -> Option<PageTableQueryResult> {
        if self.va >= self.barrier_va.end {
            return None;
        }
        loop {
            let level = self.level;
            let va = self.va;
            let map_prop = self.read_cur_pte_prop();
            match self.cur_child().clone() {
                Child::Frame(frame) => {
                    self.move_forward();
                    return Some(PageTableQueryResult::Mapped {
                        va,
                        frame,
                        prop: map_prop,
                    });
                }
                Child::PageTable(_) => {
                    // Go down if it's not a last node.
                    self.level_down(None);
                    continue;
                }
                Child::Untracked(pa) => {
                    self.move_forward();
                    return Some(PageTableQueryResult::MappedUntyped {
                        va,
                        pa,
                        len: page_size::<C>(level),
                        prop: map_prop,
                    });
                }
                Child::None => {
                    self.move_forward();
                    return Some(PageTableQueryResult::NotMapped {
                        va,
                        len: page_size::<C>(level),
                    });
                }
            }
        }
    }

    /// Consume itself and leak the root guard for the caller if it locked the root level.
    ///
    /// It is useful when the caller wants to keep the root guard while the cursor should be dropped.
    pub(super) fn leak_root_guard(mut self) -> Option<ArcSpinLockGuard<PageTableFrame<E, C>>> {
        if self.guard_level != C::NR_LEVELS {
            return None;
        }
        while self.level < C::NR_LEVELS {
            self.level_up();
        }
        self.guards[0].take()
        // Ok to drop self here because we ensure not to access the page table if the current
        // level is the root level when running the dropping method.
    }

    /// Traverse forward in the current level to the next PTE.
    ///
    /// If reached the end of a page table frame, it leads itself up to the next frame of the parent
    /// frame if possible.
    fn move_forward(&mut self) {
        let page_size = page_size::<C>(self.level);
        let next_va = self.va.align_down(page_size) + page_size;
        while self.level < self.guard_level && pte_index::<C>(next_va, self.level) == 0 {
            self.level_up();
        }
        self.va = next_va;
    }

    /// Go up a level. We release the current frame if it has no mappings since the cursor only moves
    /// forward. And if needed we will do the final cleanup using this method after re-walk when the
    /// cursor is dropped.
    ///
    /// This method requires locks acquired before calling it. The discarded level will be unlocked.
    fn level_up(&mut self) {
        #[cfg(feature = "page_table_recycle")]
        let last_node_all_unmapped = self.cur_node().nr_valid_children() == 0;
        self.guards[C::NR_LEVELS - self.level] = None;
        self.level += 1;
        #[cfg(feature = "page_table_recycle")]
        {
            let can_release_child =
                TypeId::of::<M>() == TypeId::of::<KernelMode>() && self.level < C::NR_LEVELS;
            if can_release_child && last_node_all_unmapped {
                let idx = self.cur_idx();
                self.cur_node_mut().set_child(idx, Child::None, None, false);
            }
        }
    }

    /// A level down operation during traversal. It may create a new child frame if the
    /// current frame does not have one. It may also split an untyped huge page into
    /// smaller pages if we have an end address within the next mapped untyped huge page.
    ///
    /// If creation may happen the map property of intermediate level `prop` should be
    /// passed in correctly. Whether the map property matters in an intermediate
    /// level is architecture-dependent.
    ///
    /// Also, the staticness of the page table is guaranteed if the caller make sure
    /// that there is a child node for the current node.
    fn level_down(&mut self, prop: Option<PageProperty>) {
        debug_assert!(self.level > 1);
        // Check if the child frame exists.
        let nxt_lvl_frame = {
            let idx = pte_index::<C>(self.va, self.level);
            let child = self.cur_child();
            if let Child::PageTable(nxt_lvl_frame) = child {
                Some(nxt_lvl_frame.clone())
            } else {
                None
            }
        };
        // Create a new child frame if it does not exist. Sure it could be done only if
        // it is allowed to modify the page table.
        let nxt_lvl_frame = nxt_lvl_frame.unwrap_or_else(|| {
            // If it already maps an untyped huge page, we should split it.
            if self.cur_child().is_untyped() {
                let level = self.level;
                let idx = self.cur_idx();
                self.cur_node_mut().split_untracked_huge(level, idx);
                let Child::PageTable(nxt_lvl_frame) = self.cur_child() else {
                    unreachable!()
                };
                nxt_lvl_frame.clone()
            } else if self.cur_child().is_none() {
                let new_frame = Arc::new(SpinLock::new(PageTableFrame::<E, C>::new()));
                let idx = self.cur_idx();
                self.cur_node_mut().set_child(
                    idx,
                    Child::PageTable(new_frame.clone()),
                    prop,
                    false,
                );
                new_frame
            } else {
                panic!("Trying to level down when it is mapped to a typed frame");
            }
        });
        self.guards[C::NR_LEVELS - self.level + 1] = Some(nxt_lvl_frame.lock_arc());
        self.level -= 1;
    }

    fn cur_node(&self) -> &ArcSpinLockGuard<PageTableFrame<E, C>> {
        self.guards[C::NR_LEVELS - self.level].as_ref().unwrap()
    }

    fn cur_node_mut(&mut self) -> &mut ArcSpinLockGuard<PageTableFrame<E, C>> {
        self.guards[C::NR_LEVELS - self.level].as_mut().unwrap()
    }

    fn cur_idx(&self) -> usize {
        pte_index::<C>(self.va, self.level)
    }

    fn cur_child(&self) -> &Child<E, C> {
        self.cur_node().child(self.cur_idx())
    }

    fn read_cur_pte_prop(&self) -> PageProperty {
        self.cur_node().read_pte_prop(self.cur_idx())
    }
}

#[cfg(feature = "page_table_recycle")]
impl<M: PageTableMode, E: PageTableEntryTrait, C: PagingConstsTrait> Drop for CursorMut<'_, M, E, C>
where
    [(); nr_ptes_per_node::<C>()]:,
    [(); C::NR_LEVELS]:,
{
    fn drop(&mut self) {
        // Recycle what we can recycle now.
        while self.level < self.guard_level {
            self.level_up();
        }
        // No need to do further cleanup if it is the root node or
        // there are mappings left.
        if self.level == self.guard_level || self.cur_node().nr_valid_children() != 0 {
            return;
        }
        // Drop the lock on the guard level.
        self.guards[C::NR_LEVELS - self.guard_level] = None;
        // Re-walk the page table to retreive the locks.
        self.guards[0] = Some(self.pt.root_frame.lock_arc());
        self.level = C::NR_LEVELS;
        // Another cursor can unmap the guard level node before this cursor
        // is dropped, we can just do our best here when re-walking.
        while self.level > self.guard_level && self.cur_child().is_pt() {
            self.level_down(None);
        }
        // Doing final cleanup by [`CursorMut::level_up`] to the root.
        while self.level < C::NR_LEVELS {
            self.level_up();
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) enum PageTableQueryResult {
    NotMapped {
        va: Vaddr,
        len: usize,
    },
    Mapped {
        va: Vaddr,
        frame: VmFrame,
        prop: PageProperty,
    },
    MappedUntyped {
        va: Vaddr,
        pa: Paddr,
        len: usize,
        prop: PageProperty,
    },
}

/// The read-only cursor for traversal over the page table.
///
/// It implements the `Iterator` trait to provide a convenient way to query over the page table.
pub(crate) struct Cursor<'a, M: PageTableMode, E: PageTableEntryTrait, C: PagingConstsTrait>
where
    [(); nr_ptes_per_node::<C>()]:,
    [(); C::NR_LEVELS]:,
{
    inner: CursorMut<'a, M, E, C>,
}

impl<'a, M: PageTableMode, E: PageTableEntryTrait, C: PagingConstsTrait> Cursor<'a, M, E, C>
where
    [(); nr_ptes_per_node::<C>()]:,
    [(); C::NR_LEVELS]:,
{
    pub(super) fn new(
        pt: &'a PageTable<M, E, C>,
        va: &Range<Vaddr>,
    ) -> Result<Self, PageTableError> {
        CursorMut::new(pt, va).map(|inner| Self { inner })
    }
}

impl<'a, M: PageTableMode, E: PageTableEntryTrait, C: PagingConstsTrait> Iterator
    for Cursor<'a, M, E, C>
where
    [(); nr_ptes_per_node::<C>()]:,
    [(); C::NR_LEVELS]:,
{
    type Item = PageTableQueryResult;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.query()
    }
}
