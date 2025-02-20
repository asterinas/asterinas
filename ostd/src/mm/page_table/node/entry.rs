// SPDX-License-Identifier: MPL-2.0

//! This module provides accessors to the page table entries in a node.

use super::{Child, MapTrackingStatus, PageTableEntryTrait, PageTableNode};
use crate::mm::{nr_subpage_per_huge, page_prop::PageProperty, page_size, PagingConstsTrait};

/// A view of an entry in a page table node.
///
/// It can be borrowed from a node using the [`PageTableNode::entry`] method.
///
/// This is a static reference to an entry in a node that does not account for
/// a dynamic reference count to the child. It can be used to create a owned
/// handle, which is a [`Child`].
pub(in crate::mm) struct Entry<'a, E: PageTableEntryTrait, C: PagingConstsTrait>
where
    [(); C::NR_LEVELS as usize]:,
{
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
    node: &'a mut PageTableNode<E, C>,
}

impl<'a, E: PageTableEntryTrait, C: PagingConstsTrait> Entry<'a, E, C>
where
    [(); C::NR_LEVELS as usize]:,
{
    /// Returns if the entry does not map to anything.
    pub(in crate::mm) fn is_none(&self) -> bool {
        !self.pte.is_present()
    }

    /// Returns if the entry maps to a page table node.
    pub(in crate::mm) fn is_node(&self) -> bool {
        self.pte.is_present() && !self.pte.is_last(self.node.level())
    }

    /// Gets a owned handle to the child.
    pub(in crate::mm) fn to_owned(&self) -> Child<E, C> {
        // SAFETY: The entry structure represents an existent entry with the
        // right node information.
        unsafe { Child::clone_from_pte(&self.pte, self.node.level(), self.node.is_tracked()) }
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
    pub(in crate::mm) fn replace(self, new_child: Child<E, C>) -> Child<E, C> {
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

        // SAFETY:
        //  1. The index is within the bounds.
        //  2. The new PTE is compatible with the page table node, as asserted above.
        unsafe { self.node.write_pte(self.idx, new_child.into_pte()) };

        old_child
    }

    /// Splits the entry to smaller pages if it maps to a untracked huge page.
    ///
    /// If the entry does map to a untracked huge page, it is split into smaller
    /// pages mapped by a child page table node. The new child page table node
    /// is returned.
    ///
    /// If the entry does not map to a untracked huge page, the method returns
    /// `None`.
    pub(in crate::mm) fn split_if_untracked_huge(self) -> Option<PageTableNode<E, C>> {
        let level = self.node.level();

        if !(self.pte.is_last(level)
            && level > 1
            && self.node.is_tracked() == MapTrackingStatus::Untracked)
        {
            return None;
        }

        let pa = self.pte.paddr();
        let prop = self.pte.prop();

        let mut new_page = PageTableNode::<E, C>::alloc(level - 1, MapTrackingStatus::Untracked);
        for i in 0..nr_subpage_per_huge::<C>() {
            let small_pa = pa + i * page_size::<C>(level - 1);
            let _ = new_page
                .entry(i)
                .replace(Child::Untracked(small_pa, level - 1, prop));
        }

        let _ = self.replace(Child::PageTable(new_page.clone_raw()));

        Some(new_page)
    }

    /// Create a new entry at the node.
    ///
    /// # Safety
    ///
    /// The caller must ensure that the index is within the bounds of the node.
    pub(super) unsafe fn new_at(node: &'a mut PageTableNode<E, C>, idx: usize) -> Self {
        // SAFETY: The index is within the bound.
        let pte = unsafe { node.read_pte(idx) };
        Self { pte, idx, node }
    }

    /// Get the physical address of the entry.
    ///
    /// # Safety
    ///
    /// The caller must ensure that the entry is present.
    #[cfg(ktest)]
    pub fn paddr(&self) -> usize {
        self.pte.paddr()
    }
}
