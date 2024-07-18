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
//!
//! ## Validity
//!
//! The page table cursor API will guarentee that the page table, as a data
//! structure, whose occupied memory will not suffer from data races. This is
//! ensured by the page table lock protocol. In other words, any operations
//! provided by the APIs (as long as safety requirements are met) will not
//! break the page table data structure (or other memory).
//!
//! However, the page table cursor creation APIs, [`CursorMut::new`] or
//! [`Cursor::new`], do not guarantee exclusive access to the virtual address
//! area you claim. From the lock protocol, you can see that there are chances
//! to create 2 cursors that claim the same virtual address range (one covers
//! another). In this case, the greater cursor may block if it wants to modify
//! the page table entries covered by the smaller cursor. Also, if the greater
//! cursor destructs the smaller cursor's parent page table node, it won't block
//! and the smaller cursor's change will not be visible. The user of the page
//! table cursor should add additional entry point checks to prevent these defined
//! behaviors if they are not wanted.

use core::{any::TypeId, marker::PhantomData, ops::Range};

use align_ext::AlignExt;

use super::{
    page_size, pte_index, Child, KernelMode, PageTable, PageTableEntryTrait, PageTableError,
    PageTableMode, PageTableNode, PagingConstsTrait, PagingLevel, UserMode,
};
use crate::mm::{page::DynPage, Paddr, PageProperty, Vaddr};

#[derive(Clone, Debug)]
pub enum PageTableQueryResult {
    NotMapped {
        va: Vaddr,
        len: usize,
    },
    Mapped {
        va: Vaddr,
        page: DynPage,
        prop: PageProperty,
    },
    #[allow(dead_code)]
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
pub struct Cursor<'a, M: PageTableMode, E: PageTableEntryTrait, C: PagingConstsTrait>
where
    [(); C::NR_LEVELS as usize]:,
{
    /// The lock guards of the cursor. The level 1 page table lock guard is at
    /// index 0, and the level N page table lock guard is at index N - 1.
    ///
    /// When destructing the cursor, the locks will be released in the order
    /// from low to high, exactly the reverse order of the aquisition.
    /// This behavior is ensured by the default drop implementation of Rust:
    /// <https://doc.rust-lang.org/reference/destructors.html>.
    guards: [Option<PageTableNode<E, C>>; C::NR_LEVELS as usize],
    /// The level of the page table that the cursor points to.
    level: PagingLevel,
    /// From `guard_level` to `level`, the locks are held in `guards`.
    guard_level: PagingLevel,
    /// The current virtual address that the cursor points to.
    va: Vaddr,
    /// The virtual address range that is locked.
    barrier_va: Range<Vaddr>,
    phantom: PhantomData<&'a PageTable<M, E, C>>,
}

impl<'a, M: PageTableMode, E: PageTableEntryTrait, C: PagingConstsTrait> Cursor<'a, M, E, C>
where
    [(); C::NR_LEVELS as usize]:,
{
    /// Creates a cursor claiming the read access for the given range.
    ///
    /// The cursor created will only be able to query or jump within the given
    /// range. Out-of-bound accesses will result in panics or errors as return values,
    /// depending on the access method.
    ///
    /// Note that this function does not ensure exclusive access to the claimed
    /// virtual address range. The accesses using this cursor may block or fail.
    pub fn new(pt: &'a PageTable<M, E, C>, va: &Range<Vaddr>) -> Result<Self, PageTableError> {
        if !M::covers(va) {
            return Err(PageTableError::InvalidVaddrRange(va.start, va.end));
        }
        if va.start % C::BASE_PAGE_SIZE != 0 || va.end % C::BASE_PAGE_SIZE != 0 {
            return Err(PageTableError::UnalignedVaddr);
        }

        // Create a guard array that only hold the root node lock.
        let guards = core::array::from_fn(|i| {
            if i == (C::NR_LEVELS - 1) as usize {
                Some(pt.root.clone_shallow().lock())
            } else {
                None
            }
        });
        let mut cursor = Self {
            guards,
            level: C::NR_LEVELS,
            guard_level: C::NR_LEVELS,
            va: va.start,
            barrier_va: va.clone(),
            phantom: PhantomData,
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
            if !level_too_high {
                break;
            }

            let cur_pte = cursor.read_cur_pte();
            if !cur_pte.is_present() || cur_pte.is_last(cursor.level) {
                break;
            }

            cursor.level_down();

            // Release the guard of the previous (upper) level.
            cursor.guards[cursor.level as usize] = None;
            cursor.guard_level -= 1;
        }

        Ok(cursor)
    }

    /// Gets the information of the current slot.
    pub fn query(&mut self) -> Result<PageTableQueryResult, PageTableError> {
        if self.va >= self.barrier_va.end {
            return Err(PageTableError::InvalidVaddr(self.va));
        }

        loop {
            let level = self.level;
            let va = self.va;

            let pte = self.read_cur_pte();
            if !pte.is_present() {
                return Ok(PageTableQueryResult::NotMapped {
                    va,
                    len: page_size::<C>(level),
                });
            }
            if !pte.is_last(level) {
                self.level_down();
                continue;
            }

            match self.cur_child() {
                Child::Page(page) => {
                    return Ok(PageTableQueryResult::Mapped {
                        va,
                        page,
                        prop: pte.prop(),
                    });
                }
                Child::Untracked(pa) => {
                    return Ok(PageTableQueryResult::MappedUntracked {
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

    /// Traverses forward in the current level to the next PTE.
    ///
    /// If reached the end of a page table node, it leads itself up to the next page of the parent
    /// page if possible.
    pub(in crate::mm) fn move_forward(&mut self) {
        let page_size = page_size::<C>(self.level);
        let next_va = self.va.align_down(page_size) + page_size;
        while self.level < self.guard_level && pte_index::<C>(next_va, self.level) == 0 {
            self.level_up();
        }
        self.va = next_va;
    }

    /// Jumps to the given virtual address.
    ///
    /// # Panics
    ///
    /// This method panics if the address is out of the range where the cursor is required to operate,
    /// or has bad alignment.
    pub fn jump(&mut self, va: Vaddr) {
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

    pub fn virt_addr(&self) -> Vaddr {
        self.va
    }

    /// Goes up a level. We release the current page if it has no mappings since the cursor only moves
    /// forward. And if needed we will do the final cleanup using this method after re-walk when the
    /// cursor is dropped.
    ///
    /// This method requires locks acquired before calling it. The discarded level will be unlocked.
    fn level_up(&mut self) {
        self.guards[(self.level - 1) as usize] = None;
        self.level += 1;

        // TODO: Drop page tables if page tables become empty.
    }

    /// Goes down a level assuming a child page table exists.
    fn level_down(&mut self) {
        debug_assert!(self.level > 1);

        let Child::PageTable(nxt_lvl_ptn) = self.cur_child() else {
            panic!("Trying to level down when it is not mapped to a page table");
        };

        let nxt_lvl_ptn_locked = nxt_lvl_ptn.lock();

        self.level -= 1;
        debug_assert_eq!(self.level, nxt_lvl_ptn_locked.level());

        self.guards[(self.level - 1) as usize] = Some(nxt_lvl_ptn_locked);
    }

    fn cur_node(&self) -> &PageTableNode<E, C> {
        self.guards[(self.level - 1) as usize].as_ref().unwrap()
    }

    fn cur_idx(&self) -> usize {
        pte_index::<C>(self.va, self.level)
    }

    fn cur_child(&self) -> Child<E, C> {
        self.cur_node()
            .child(self.cur_idx(), self.in_tracked_range())
    }

    fn read_cur_pte(&self) -> E {
        self.cur_node().read_pte(self.cur_idx())
    }

    /// Tells if the current virtual range must contain untracked mappings.
    ///
    /// _Tracked mappings_ means that the mapped physical addresses (in PTEs) points to pages
    /// tracked by the metadata system. _Tracked mappings_ must be created with page handles.
    /// While _untracked mappings_ solely maps to plain physical addresses.
    ///
    /// In the kernel mode, this is aligned with the definition in [`crate::mm::kspace`].
    /// Only linear mappings in the kernel should be considered as untracked mappings.
    ///
    /// All mappings in the user mode are tracked. And all mappings in the IOMMU
    /// page table are untracked.
    fn in_tracked_range(&self) -> bool {
        TypeId::of::<M>() == TypeId::of::<UserMode>()
            || TypeId::of::<M>() == TypeId::of::<KernelMode>()
                && !crate::mm::kspace::LINEAR_MAPPING_VADDR_RANGE.contains(&self.va)
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
        if result.is_ok() {
            self.move_forward();
        }
        result.ok()
    }
}

/// The cursor of a page table that is capable of map, unmap or protect pages.
///
/// Also, it has all the capabilities of a [`Cursor`]. A virtual address range
/// in a page table can only be accessed by one cursor whether it is mutable or not.
#[derive(Debug)]
pub struct CursorMut<'a, M: PageTableMode, E: PageTableEntryTrait, C: PagingConstsTrait>(
    Cursor<'a, M, E, C>,
)
where
    [(); C::NR_LEVELS as usize]:;

impl<'a, M: PageTableMode, E: PageTableEntryTrait, C: PagingConstsTrait> CursorMut<'a, M, E, C>
where
    [(); C::NR_LEVELS as usize]:,
{
    /// Creates a cursor claiming the write access for the given range.
    ///
    /// The cursor created will only be able to map, query or jump within the given
    /// range. Out-of-bound accesses will result in panics or errors as return values,
    /// depending on the access method.
    ///
    /// Note that this function, the same as [`Cursor::new`], does not ensure exclusive
    /// access to the claimed virtual address range. The accesses using this cursor may
    /// block or fail.
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
    pub fn jump(&mut self, va: Vaddr) {
        self.0.jump(va)
    }

    /// Gets the current virtual address.
    pub fn virt_addr(&self) -> Vaddr {
        self.0.virt_addr()
    }

    /// Gets the information of the current slot.
    pub fn query(&mut self) -> Result<PageTableQueryResult, PageTableError> {
        self.0.query()
    }

    /// Maps the range starting from the current address to a [`DynPage`].
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
    pub unsafe fn map(&mut self, page: DynPage, prop: PageProperty) {
        let end = self.0.va + page.size();
        assert!(end <= self.0.barrier_va.end);
        debug_assert!(self.0.in_tracked_range());

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
        debug_assert_eq!(self.0.level, page.level());

        // Map the current page.
        let idx = self.0.cur_idx();
        self.cur_node_mut().set_child_page(idx, page, prop);
        self.0.move_forward();
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
            debug_assert!(!self.0.in_tracked_range());
            let idx = self.0.cur_idx();
            self.cur_node_mut().set_child_untracked(idx, pa, prop);

            let level = self.0.level;
            pa += page_size::<C>(level);
            self.0.move_forward();
        }
    }

    /// Unmaps the range starting from the current address with the given length of virtual address.
    ///
    /// # Safety
    ///
    /// The caller should ensure that the range being unmapped does not affect kernel's memory safety.
    ///
    /// # Panics
    ///
    /// This function will panic if:
    ///  - the range to be unmapped is out of the range where the cursor is required to operate;
    ///  - the range covers only a part of a page.
    pub unsafe fn unmap(&mut self, len: usize) {
        let end = self.0.va + len;
        assert!(end <= self.0.barrier_va.end);
        assert!(end % C::BASE_PAGE_SIZE == 0);

        while self.0.va < end {
            let cur_pte = self.0.read_cur_pte();
            let is_tracked = self.0.in_tracked_range();

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
                } else if !is_tracked {
                    self.level_down_split();
                } else {
                    unreachable!();
                }
                continue;
            }

            // Unmap the current page.
            let idx = self.0.cur_idx();
            self.cur_node_mut().unset_child(idx, is_tracked);

            self.0.move_forward();
        }
    }

    /// Applies the given operation to all the mappings within the range.
    ///
    /// The funtction will return an error if it is not allowed to protect an invalid range and
    /// it does so, or if the range to be protected only covers a part of a page.
    ///
    /// # Safety
    ///
    /// The caller should ensure that the range being protected does not affect kernel's memory safety.
    ///
    /// # Panics
    ///
    /// This function will panic if:
    ///  - the range to be protected is out of the range where the cursor is required to operate.
    pub unsafe fn protect(
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
            if !self.0.in_tracked_range() && vaddr_not_fit {
                self.level_down_split();
                continue;
            } else if vaddr_not_fit {
                return Err(PageTableError::ProtectingPartial);
            }

            let mut pte_prop = cur_pte.prop();
            op(&mut pte_prop);

            let idx = self.0.cur_idx();
            self.cur_node_mut().protect(idx, pte_prop);

            self.0.move_forward();
        }
        Ok(())
    }

    /// Consumes itself and leak the root guard for the caller if it locked the root level.
    ///
    /// It is useful when the caller wants to keep the root guard while the cursor should be dropped.
    pub(super) fn leak_root_guard(mut self) -> Option<PageTableNode<E, C>> {
        if self.0.guard_level != C::NR_LEVELS {
            return None;
        }

        while self.0.level < C::NR_LEVELS {
            self.0.level_up();
        }

        self.0.guards[(C::NR_LEVELS - 1) as usize].take()

        // Ok to drop the cursor here because we ensure not to access the page table if the current
        // level is the root level when running the dropping method.
    }

    /// Goes down a level assuming the current slot is absent.
    ///
    /// This method will create a new child page table node and go down to it.
    fn level_down_create(&mut self) {
        debug_assert!(self.0.level > 1);
        let new_node = PageTableNode::<E, C>::alloc(self.0.level - 1);
        let idx = self.0.cur_idx();
        let is_tracked = self.0.in_tracked_range();
        self.cur_node_mut()
            .set_child_pt(idx, new_node.clone_raw(), is_tracked);
        self.0.level -= 1;
        self.0.guards[(self.0.level - 1) as usize] = Some(new_node);
    }

    /// Goes down a level assuming the current slot is an untracked huge page.
    ///
    /// This method will split the huge page and go down to the next level.
    fn level_down_split(&mut self) {
        debug_assert!(self.0.level > 1);
        debug_assert!(!self.0.in_tracked_range());

        let idx = self.0.cur_idx();
        self.cur_node_mut().split_untracked_huge(idx);

        let Child::PageTable(new_node) = self.0.cur_child() else {
            unreachable!();
        };
        self.0.level -= 1;
        self.0.guards[(self.0.level - 1) as usize] = Some(new_node.lock());
    }

    fn cur_node_mut(&mut self) -> &mut PageTableNode<E, C> {
        self.0.guards[(self.0.level - 1) as usize].as_mut().unwrap()
    }
}
