// SPDX-License-Identifier: MPL-2.0

//! This module provides accessors to the page table entries in a node.

use super::{Child, MapTrackingStatus, PageTableEntryTrait, PageTableGuard, PageTableNode};
use crate::mm::{nr_subpage_per_huge, page_prop::PageProperty, page_size, PagingConstsTrait};

/// A view of an entry in a page table node.
///
/// It can be borrowed from a node using the [`PageTableGuard::entry`] method.
///
/// This is a static reference to an entry in a node that does not account for
/// a dynamic reference count to the child. It can be used to create a owned
/// handle, which is a [`Child`].
// Note that
pub(in crate::mm) struct Entry<'guard, 'pt, E: PageTableEntryTrait, C: PagingConstsTrait> {
    /// The page table entry.
    ///
    /// We store the page table entry here to optimize the number of reads from
    /// the node. We cannot hold a `&mut E` reference to the entry because that
    /// other CPUs may modify the memory location for accessed/dirty bits. Such
    /// accesses will violate the aliasing rules of Rust and cause undefined
    /// behaviors.
    pte: E,
    /// The index of the entry in the node.
    idx: usize,
    /// The node that contains the entry.
    node: &'guard mut PageTableGuard<'pt, E, C>,
}

impl<'guard, 'pt, E: PageTableEntryTrait, C: PagingConstsTrait> Entry<'guard, 'pt, E, C> {
    /// Returns if the entry does not map to anything.
    pub(in crate::mm) fn is_none(&self) -> bool {
        !self.pte.is_present()
    }

    /// Returns if the entry maps to a page table node.
    pub(in crate::mm) fn is_node(&self) -> bool {
        self.pte.is_present() && !self.pte.is_last(self.node.level())
    }

    /// Gets a reference to the child.
    pub(in crate::mm) fn to_ref(&self) -> Child<'_, E, C> {
        // SAFETY: The entry structure represents an existent entry with the
        // right node information.
        unsafe { Child::ref_from_pte(&self.pte, self.node.level(), self.node.is_tracked()) }
    }

    /// Operates on the mapping properties of the entry.
    ///
    /// It only modifies the properties if the entry is present.
    pub(in crate::mm) fn protect(&mut self, op: &mut impl FnMut(&mut PageProperty)) {
        if !self.pte.is_present() {
            return;
        }

        let prop = self.pte.prop();
        let mut new_prop = prop;
        op(&mut new_prop);

        if prop == new_prop {
            return;
        }

        self.pte.set_prop(new_prop);

        // SAFETY:
        //  1. The index is within the bounds.
        //  2. We replace the PTE with a new one, which differs only in
        //     `PageProperty`, so it is still compatible with the current
        //     page table node.
        unsafe { self.node.write_pte(self.idx, self.pte) };
    }

    /// Replaces the entry with a new child.
    ///
    /// The old child is returned.
    ///
    /// # Panics
    ///
    /// The method panics if the given child is not compatible with the node.
    /// The compatibility is specified by the [`Child::is_compatible`].
    pub(in crate::mm) fn replace(&mut self, new_child: Child<E, C>) -> Child<E, C> {
        assert!(new_child.is_compatible(self.node.level(), self.node.is_tracked()));

        // SAFETY: The entry structure represents an existent entry with the
        // right node information. The old PTE is overwritten by the new child
        // so that it is not used anymore.
        let old_child =
            unsafe { Child::from_pte(self.pte, self.node.level(), self.node.is_tracked()) };

        if old_child.is_none() && !new_child.is_none() {
            *self.node.nr_children_mut() += 1;
        } else if !old_child.is_none() && new_child.is_none() {
            *self.node.nr_children_mut() -= 1;
        }

        let new_pte = new_child.into_pte();

        // SAFETY:
        //  1. The index is within the bounds.
        //  2. The new PTE is compatible with the page table node, as asserted above.
        unsafe { self.node.write_pte(self.idx, new_pte) };

        self.pte = new_pte;

        old_child
    }

    /// Allocates a new child page table node and replaces the entry with it.
    ///
    /// If the old entry is not none, the operation will fail and return `None`.
    /// Otherwise, the lock guard of the new child page table node is returned.
    pub(in crate::mm::page_table) fn alloc_if_none(
        &mut self,
        new_pt_is_tracked: MapTrackingStatus,
    ) -> Option<PageTableGuard<'pt, E, C>> {
        if !self.is_none() {
            return None;
        }

        let level = self.node.level();
        let new_page = PageTableNode::<E, C>::alloc(level - 1, new_pt_is_tracked);

        let guard_addr = new_page.lock().into_raw_paddr();

        // SAFETY:
        //  1. The index is within the bounds.
        //  2. The new PTE is compatible with the page table node.
        unsafe {
            self.node
                .write_pte(self.idx, Child::PageTable(new_page).into_pte())
        };

        *self.node.nr_children_mut() += 1;

        // SAFETY: The resulting guard lifetime (`'a`) is no shorter than the
        // lifetime of the current entry (`'a`), because we store the allocated
        // page table in the current node.
        Some(unsafe { PageTableGuard::from_raw_paddr(guard_addr) })
    }

    /// Splits the entry to smaller pages if it maps to a untracked huge page.
    ///
    /// If the entry does map to a untracked huge page, it is split into smaller
    /// pages mapped by a child page table node. The new child page table node
    /// is returned.
    ///
    /// If the entry does not map to a untracked huge page, the method returns
    /// `None`.
    pub(in crate::mm::page_table) fn split_if_untracked_huge(
        &mut self,
    ) -> Option<PageTableGuard<'pt, E, C>> {
        let level = self.node.level();

        if !(self.pte.is_last(level)
            && level > 1
            && self.node.is_tracked() == MapTrackingStatus::Untracked)
        {
            return None;
        }

        let pa = self.pte.paddr();
        let prop = self.pte.prop();

        let new_page = PageTableNode::<E, C>::alloc(level - 1, MapTrackingStatus::Untracked);
        let mut guard = new_page.lock();

        for i in 0..nr_subpage_per_huge::<C>() {
            let small_pa = pa + i * page_size::<C>(level - 1);
            let mut entry = guard.entry(i);
            let old = entry.replace(Child::Untracked(small_pa, level - 1, prop));
            debug_assert!(old.is_none());
        }

        let guard_addr = guard.into_raw_paddr();

        // SAFETY:
        //  1. The index is within the bounds.
        //  2. The new PTE is compatible with the page table node.
        unsafe {
            self.node
                .write_pte(self.idx, Child::PageTable(new_page).into_pte())
        };

        // SAFETY: The resulting guard lifetime (`'a`) is no shorter than the
        // lifetime of the current entry (`'a`), because we store the allocated
        // page table in the current node.
        Some(unsafe { PageTableGuard::from_raw_paddr(guard_addr) })
    }

    /// Create a new entry at the node with guard.
    ///
    /// # Safety
    ///
    /// The caller must ensure that the index is within the bounds of the node.
    pub(super) unsafe fn new_at(guard: &'guard mut PageTableGuard<'pt, E, C>, idx: usize) -> Self {
        // SAFETY: The index is within the bound.
        let pte = unsafe { guard.read_pte(idx) };
        Self {
            pte,
            idx,
            node: guard,
        }
    }
}
