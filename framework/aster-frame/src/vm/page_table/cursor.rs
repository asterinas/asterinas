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

use core::{any::TypeId, ops::Range};

use align_ext::AlignExt;

use super::{
    page_size, pte_index, Child, KernelMode, PageTable, PageTableEntryTrait, PageTableError,
    PageTableFrame, PageTableMode, PagingConstsTrait, PagingLevel,
};
use crate::vm::{Paddr, PageProperty, Vaddr, VmFrame};

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
    MappedUntracked {
        va: Vaddr,
        pa: Paddr,
        len: usize,
        prop: PageProperty,
    },
}

/// The cursor for traversal over the page table.
///
/// A slot is a PTE at any levels, which correspond to a certain virtual
/// memory range sized by the "page size" of the current level.
///
/// A cursor is able to move to the next slot, to read page properties,
/// and even to jump to a virtual address directly. We use a guard stack to
/// simulate the recursion, and adpot a page table locking protocol to
/// provide concurrency.
#[derive(Debug)]
pub(crate) struct Cursor<'a, M: PageTableMode, E: PageTableEntryTrait, C: PagingConstsTrait>
where
    [(); C::NR_LEVELS as usize]:,
{
    pt: &'a PageTable<M, E, C>,
    guards: [Option<PageTableFrame<E, C>>; C::NR_LEVELS as usize],
    level: PagingLevel,       // current level
    guard_level: PagingLevel, // from guard_level to level, the locks are held
    va: Vaddr,                // current virtual address
    barrier_va: Range<Vaddr>, // virtual address range that is locked
}

impl<'a, M: PageTableMode, E: PageTableEntryTrait, C: PagingConstsTrait> Cursor<'a, M, E, C>
where
    [(); C::NR_LEVELS as usize]:,
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
                Some(pt.root.copy_handle().lock())
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
            let cur_pte = cursor.read_cur_pte();
            let level_too_high = {
                let start_idx = pte_index::<C>(va.start, cursor.level);
                let end_idx = pte_index::<C>(va.end - 1, cursor.level);
                start_idx == end_idx
            };
            if !level_too_high || !cur_pte.is_present() || cur_pte.is_last(cursor.level) {
                break;
            }
            cursor.level_down();
            // Release the guard of the previous level.
            cursor.guards[(C::NR_LEVELS - cursor.level) as usize - 1] = None;
            cursor.guard_level -= 1;
        }
        Ok(cursor)
    }

    /// Get the information of the current slot.
    pub(crate) fn query(&mut self) -> Option<PageTableQueryResult> {
        if self.va >= self.barrier_va.end {
            return None;
        }
        loop {
            let level = self.level;
            let va = self.va;
            let pte = self.read_cur_pte();
            if !pte.is_present() {
                return Some(PageTableQueryResult::NotMapped {
                    va,
                    len: page_size::<C>(level),
                });
            }
            if !pte.is_last(level) {
                self.level_down();
                continue;
            }
            match self.cur_child() {
                Child::Frame(frame) => {
                    return Some(PageTableQueryResult::Mapped {
                        va,
                        frame,
                        prop: pte.prop(),
                    });
                }
                Child::Untracked(pa) => {
                    return Some(PageTableQueryResult::MappedUntracked {
                        va,
                        pa,
                        len: page_size::<C>(level),
                        prop: pte.prop(),
                    });
                }
                Child::None | Child::PageTable(_) => {
                    unreachable!(); // Already checked with the PTE.
                }
            }
        }
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
        self.guards[(C::NR_LEVELS - self.level) as usize] = None;
        self.level += 1;
        #[cfg(feature = "page_table_recycle")]
        {
            let can_release_child =
                TypeId::of::<M>() == TypeId::of::<KernelMode>() && self.level < C::NR_LEVELS;
            if can_release_child && last_node_all_unmapped {
                let idx = self.cur_idx();
                let untracked = self.in_untracked_range();
                self.cur_node_mut().unset_child(idx, false, untracked);
            }
        }
    }

    /// Go down a level assuming a child page table exists.
    fn level_down(&mut self) {
        debug_assert!(self.level > 1);
        let idx = pte_index::<C>(self.va, self.level);
        if let Child::PageTable(nxt_lvl_frame) = self.cur_child() {
            self.level -= 1;
            self.guards[(C::NR_LEVELS - self.level) as usize] = Some(nxt_lvl_frame.lock());
        } else {
            panic!("Trying to level down when it is not mapped to a page table");
        }
    }

    fn cur_node(&self) -> &PageTableFrame<E, C> {
        self.guards[(C::NR_LEVELS - self.level) as usize]
            .as_ref()
            .unwrap()
    }

    fn cur_idx(&self) -> usize {
        pte_index::<C>(self.va, self.level)
    }

    fn cur_child(&self) -> Child<E, C> {
        self.cur_node()
            .child(self.cur_idx(), !self.in_untracked_range())
    }

    fn read_cur_pte(&self) -> E {
        self.cur_node().read_pte(self.cur_idx())
    }

    /// Tell if the current virtual range must contain untracked mappings.
    ///
    /// In the kernel mode, this is aligned with the definition in [`crate::vm::kspace`].
    /// Only linear mappings in the kernel are considered as untracked mappings.
    ///
    /// All mappings in the user mode are tracked. And all mappings in the IOMMU
    /// page table are untracked.
    fn in_untracked_range(&self) -> bool {
        TypeId::of::<M>() == TypeId::of::<crate::arch::iommu::DeviceMode>()
            || TypeId::of::<M>() == TypeId::of::<KernelMode>()
                && !crate::vm::kspace::VMALLOC_VADDR_RANGE.contains(&self.va)
    }
}

impl<'a, M: PageTableMode, E: PageTableEntryTrait, C: PagingConstsTrait> Iterator
    for Cursor<'a, M, E, C>
where
    [(); C::NR_LEVELS as usize]:,
{
    type Item = PageTableQueryResult;

    fn next(&mut self) -> Option<Self::Item> {
        let result = self.query();
        if result.is_some() {
            self.move_forward();
        }
        result
    }
}

#[cfg(feature = "page_table_recycle")]
impl<M: PageTableMode, E: PageTableEntryTrait, C: PagingConstsTrait> Drop for Cursor<'_, M, E, C>
where
    [(); C::NR_LEVELS as usize]:,
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
        self.guards[0] = Some(self.pt.root.copy_handle().lock());
        self.level = C::NR_LEVELS;
        let cur_pte = self.read_cur_pte();
        let cur_child_is_pt = cur_pte.is_present() && !cur_pte.is_last(self.level);
        // Another cursor can unmap the guard level node before this cursor
        // is dropped, we can just do our best here when re-walking.
        while self.level > self.guard_level && cur_child_is_pt {
            self.level_down();
        }
        // Doing final cleanup by [`CursorMut::level_up`] to the root.
        while self.level < C::NR_LEVELS {
            self.level_up();
        }
    }
}

/// The cursor of a page table that is capable of map, unmap or protect pages.
///
/// Also, it has all the capabilities of a [`Cursor`]. A virtual address range
/// in a page table can only be accessed by one cursor whether it is mutable or not.
#[derive(Debug)]
pub(crate) struct CursorMut<'a, M: PageTableMode, E: PageTableEntryTrait, C: PagingConstsTrait>(
    Cursor<'a, M, E, C>,
)
where
    [(); C::NR_LEVELS as usize]:;

impl<'a, M: PageTableMode, E: PageTableEntryTrait, C: PagingConstsTrait> CursorMut<'a, M, E, C>
where
    [(); C::NR_LEVELS as usize]:,
{
    pub(super) fn new(
        pt: &'a PageTable<M, E, C>,
        va: &Range<Vaddr>,
    ) -> Result<Self, PageTableError> {
        Cursor::new(pt, va).map(|inner| Self(inner))
    }

    /// Get the information of the current slot and go to the next slot.
    ///
    /// We choose not to implement `Iterator` or `IterMut` for [`CursorMut`]
    /// because the mutable cursor is indeed not an iterator.
    pub(crate) fn next(&mut self) -> Option<PageTableQueryResult> {
        self.0.next()
    }

    /// Jump to the given virtual address.
    ///
    /// It panics if the address is out of the range where the cursor is required to operate,
    /// or has bad alignment.
    pub(crate) fn jump(&mut self, va: Vaddr) {
        assert!(self.0.barrier_va.contains(&va));
        assert!(va % C::BASE_PAGE_SIZE == 0);
        loop {
            let cur_node_start = self.0.va & !(page_size::<C>(self.0.level + 1) - 1);
            let cur_node_end = cur_node_start + page_size::<C>(self.0.level + 1);
            // If the address is within the current node, we can jump directly.
            if cur_node_start <= va && va < cur_node_end {
                self.0.va = va;
                return;
            }
            // There is a corner case that the cursor is depleted, sitting at the start of the
            // next node but the next node is not locked because the parent is not locked.
            if self.0.va >= self.0.barrier_va.end && self.0.level == self.0.guard_level {
                self.0.va = va;
                return;
            }
            debug_assert!(self.0.level < self.0.guard_level);
            self.0.level_up();
        }
    }

    /// Map the range starting from the current address to a `VmFrame`.
    ///
    /// # Panic
    ///
    /// This function will panic if
    ///  - the virtual address range to be mapped is out of the range;
    ///  - the alignment of the frame is not satisfied by the virtual address;
    ///  - it is already mapped to a huge page while the caller wants to map a smaller one.
    ///
    /// # Safety
    ///
    /// The caller should ensure that the virtual range being mapped does
    /// not affect kernel's memory safety.
    pub(crate) unsafe fn map(&mut self, frame: VmFrame, prop: PageProperty) {
        let end = self.0.va + frame.size();
        assert!(end <= self.0.barrier_va.end);
        debug_assert!(!self.0.in_untracked_range());
        // Go down if not applicable.
        while self.0.level > C::HIGHEST_TRANSLATION_LEVEL
            || self.0.va % page_size::<C>(self.0.level) != 0
            || self.0.va + page_size::<C>(self.0.level) > end
        {
            let pte = self.0.read_cur_pte();
            if pte.is_present() && !pte.is_last(self.0.level) {
                self.0.level_down();
            } else if !pte.is_present() {
                self.level_down_create();
            } else {
                panic!("Mapping a smaller page in an already mapped huge page");
            }
            continue;
        }
        debug_assert_eq!(self.0.level, frame.level());
        // Map the current page.
        let idx = self.0.cur_idx();
        let level = self.0.level;
        self.cur_node_mut().set_child_frame(idx, frame, prop);
        self.0.move_forward();
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
    /// # Panic
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
    pub(crate) unsafe fn map_pa(&mut self, pa: &Range<Paddr>, prop: PageProperty) {
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
                let pte = self.0.read_cur_pte();
                if pte.is_present() && !pte.is_last(self.0.level) {
                    self.0.level_down();
                } else if !pte.is_present() {
                    self.level_down_create();
                } else {
                    self.level_down_split();
                }
                continue;
            }
            // Map the current page.
            debug_assert!(self.0.in_untracked_range());
            let idx = self.0.cur_idx();
            let level = self.0.level;
            self.cur_node_mut().set_child_untracked(idx, pa, prop);
            pa += page_size::<C>(level);
            self.0.move_forward();
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
        let end = self.0.va + len;
        assert!(end <= self.0.barrier_va.end);
        assert!(end % C::BASE_PAGE_SIZE == 0);
        while self.0.va < end {
            let cur_pte = self.0.read_cur_pte();
            let untracked = self.0.in_untracked_range();

            // Skip if it is already invalid.
            if !cur_pte.is_present() {
                if self.0.va + page_size::<C>(self.0.level) > end {
                    break;
                }
                self.0.move_forward();
                continue;
            }

            // We check among the conditions that may lead to a level down.
            // We ensure not unmapping in reserved kernel shared tables or releasing it.
            let is_kernel_shared_node =
                TypeId::of::<M>() == TypeId::of::<KernelMode>() && self.0.level >= C::NR_LEVELS - 1;
            if is_kernel_shared_node
                || self.0.va % page_size::<C>(self.0.level) != 0
                || self.0.va + page_size::<C>(self.0.level) > end
            {
                if cur_pte.is_present() && !cur_pte.is_last(self.0.level) {
                    self.0.level_down();
                } else if untracked {
                    self.level_down_split();
                } else {
                    unreachable!();
                }
                continue;
            }

            // Unmap the current page.
            let idx = self.0.cur_idx();
            self.cur_node_mut().unset_child(idx, untracked);
            self.0.move_forward();
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
        allow_protect_absent: bool,
    ) -> Result<(), PageTableError> {
        let end = self.0.va + len;
        assert!(end <= self.0.barrier_va.end);
        while self.0.va < end {
            let cur_pte = self.0.read_cur_pte();
            if !cur_pte.is_present() {
                if !allow_protect_absent {
                    return Err(PageTableError::ProtectingAbsent);
                }
                self.0.move_forward();
                continue;
            }
            // Go down if it's not a last node.
            if !cur_pte.is_last(self.0.level) {
                self.0.level_down();
                continue;
            }
            // Go down if the page size is too big and we are protecting part
            // of untracked huge pages.
            let vaddr_not_fit = self.0.va % page_size::<C>(self.0.level) != 0
                || self.0.va + page_size::<C>(self.0.level) > end;
            if self.0.in_untracked_range() && vaddr_not_fit {
                self.level_down_split();
                continue;
            } else if vaddr_not_fit {
                return Err(PageTableError::ProtectingPartial);
            }
            let idx = self.0.cur_idx();
            let level = self.0.level;
            let mut pte_prop = cur_pte.prop();
            op(&mut pte_prop);
            self.cur_node_mut().protect(idx, pte_prop);
            self.0.move_forward();
        }
        Ok(())
    }

    /// Consume itself and leak the root guard for the caller if it locked the root level.
    ///
    /// It is useful when the caller wants to keep the root guard while the cursor should be dropped.
    pub(super) fn leak_root_guard(mut self) -> Option<PageTableFrame<E, C>> {
        if self.0.guard_level != C::NR_LEVELS {
            return None;
        }
        while self.0.level < C::NR_LEVELS {
            self.0.level_up();
        }
        self.0.guards[0].take()
        // Ok to drop the cursor here because we ensure not to access the page table if the current
        // level is the root level when running the dropping method.
    }

    /// Go down a level assuming the current slot is absent.
    ///
    /// This method will create a new child frame and go down to it.
    fn level_down_create(&mut self) {
        debug_assert!(self.0.level > 1);
        let new_frame = PageTableFrame::<E, C>::alloc(self.0.level - 1);
        let idx = self.0.cur_idx();
        let untracked = self.0.in_untracked_range();
        self.cur_node_mut()
            .set_child_pt(idx, new_frame.clone_raw(), untracked);
        self.0.level -= 1;
        self.0.guards[(C::NR_LEVELS - self.0.level) as usize] = Some(new_frame);
    }

    /// Go down a level assuming the current slot is an untracked huge page.
    ///
    /// This method will split the huge page and go down to the next level.
    fn level_down_split(&mut self) {
        debug_assert!(self.0.level > 1);
        debug_assert!(self.0.in_untracked_range());
        let idx = self.0.cur_idx();
        self.cur_node_mut().split_untracked_huge(idx);
        let Child::PageTable(new_frame) = self.0.cur_child() else {
            unreachable!();
        };
        self.0.level -= 1;
        self.0.guards[(C::NR_LEVELS - self.0.level) as usize] = Some(new_frame.lock());
    }

    fn cur_node_mut(&mut self) -> &mut PageTableFrame<E, C> {
        self.0.guards[(C::NR_LEVELS - self.0.level) as usize]
            .as_mut()
            .unwrap()
    }
}
