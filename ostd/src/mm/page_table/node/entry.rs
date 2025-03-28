// SPDX-License-Identifier: MPL-2.0

//! This module provides accessors to the page table entries in a node.

use super::{Child, MapTrackingStatus, PageTableEntryTrait, PageTableLock, PageTableNode};
use crate::mm::{
    nr_subpage_per_huge, page_prop::PageProperty, page_size, page_table::zeroed_pt_pool,
    vm_space::Token, PagingConstsTrait,
};

/// A view of an entry in a page table node.
///
/// It can be borrowed from a node using the [`PageTableLock::entry`] method.
///
/// This is a static reference to an entry in a node that does not account for
/// a dynamic reference count to the child. It can be used to create a owned
/// handle, which is a [`Child`].
pub(in crate::mm) struct Entry<'a, E: PageTableEntryTrait, C: PagingConstsTrait> {
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
    node: &'a mut PageTableLock<E, C>,
}

impl<'a, E: PageTableEntryTrait, C: PagingConstsTrait> Entry<'a, E, C> {
    /// Returns if the entry does not map to anything.
    pub(in crate::mm) fn is_none(&self) -> bool {
        !self.pte.is_present() && self.pte.paddr() == 0
    }

    /// Returns if the entry is marked with a token.
    pub(in crate::mm) fn is_token(&self) -> bool {
        !self.pte.is_present() && self.pte.paddr() != 0
    }

    /// Returns if the entry maps to a page table node.
    pub(in crate::mm) fn is_node(&self) -> bool {
        self.pte.is_present() && !self.pte.is_last(self.node.level())
    }

    /// Gets a owned handle to the child.
    pub(in crate::mm) fn to_owned(&self) -> Child<E, C> {
        // SAFETY: The entry structure represents an existent entry with the
        // right node information.
        unsafe { Child::ref_from_pte(&self.pte, self.node.level(), self.node.is_tracked(), true) }
    }

    /// Gets a reference to the child.
    pub(in crate::mm) fn to_ref(&self) -> Child<E, C> {
        // SAFETY: The entry structure represents an existent entry with the
        // right node information.
        unsafe { Child::ref_from_pte(&self.pte, self.node.level(), self.node.is_tracked(), false) }
    }

    /// Operates on the mapping properties of the entry.
    ///
    /// It only modifies the properties if the entry is present.
    pub(in crate::mm) fn protect(
        &mut self,
        prot_op: &mut impl FnMut(&mut PageProperty),
        token_op: &mut impl FnMut(&mut Token),
    ) {
        if self.pte.is_present() {
            // Protect a proper mapping.
            let prop = self.pte.prop();
            let mut new_prop = prop;
            prot_op(&mut new_prop);

            if prop == new_prop {
                return;
            }

            self.pte.set_prop(new_prop);
        } else {
            let paddr = self.pte.paddr();
            if paddr == 0 {
                // Not mapped.
                return;
            } else {
                // Protect a token.

                // SAFETY: The physical address was written as a valid token.
                let mut token = unsafe { Token::from_raw_inner(paddr) };
                token_op(&mut token);
                self.pte.set_paddr(token.into_raw_inner());
            }
        }

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
    pub(in crate::mm) fn split_if_untracked_huge(self) -> Option<PageTableLock<E, C>> {
        let level = self.node.level();

        if !(self.pte.is_last(level)
            && level > 1
            && self.node.is_tracked() == MapTrackingStatus::Untracked)
        {
            return None;
        }

        let pa = self.pte.paddr();
        let prop = self.pte.prop();

        let preempt_guard = crate::task::disable_preempt();
        let mut new_page =
            zeroed_pt_pool::alloc::<E, C>(&preempt_guard, level - 1, MapTrackingStatus::Untracked);
        for i in 0..nr_subpage_per_huge::<C>() {
            let small_pa = pa + i * page_size::<C>(level - 1);
            let _ = new_page
                .entry(i)
                .replace(Child::Untracked(small_pa, level - 1, prop));
        }
        let pt_paddr = new_page.into_raw_paddr();
        // SAFETY: It was forgotten at the above line.
        let _ = self.replace(Child::PageTable(unsafe {
            PageTableNode::from_raw(pt_paddr)
        }));
        // SAFETY: `pt_paddr` points to a PT that is attached to the node,
        // so that it is locked and alive.
        Some(unsafe { PageTableLock::from_raw_paddr(pt_paddr) })
    }

    /// Splits the entry into a child that is marked with a same token.
    ///
    /// This method returns [`None`] if the entry is not marked with a token or
    /// it is in the last level.
    pub(in crate::mm) fn split_if_huge_token(self) -> Option<PageTableLock<E, C>> {
        let level = self.node.level();

        if !(!self.pte.is_present() && level > 1 && self.pte.paddr() != 0) {
            return None;
        }

        // SAFETY: The physical address was written as a valid token.
        let token = unsafe { Token::from_raw_inner(self.pte.paddr()) };

        let preempt_guard = crate::task::disable_preempt();
        let mut new_page =
            zeroed_pt_pool::alloc::<E, C>(&preempt_guard, level - 1, self.node.is_tracked());
        for i in 0..nr_subpage_per_huge::<C>() {
            let _ = new_page.entry(i).replace(Child::Token(token));
        }
        let pt_paddr = new_page.into_raw_paddr();
        let _ = self.replace(Child::PageTable(unsafe {
            PageTableNode::from_raw(pt_paddr)
        }));

        Some(unsafe { PageTableLock::from_raw_paddr(pt_paddr) })
    }

    /// Create a new entry at the node.
    ///
    /// # Safety
    ///
    /// The caller must ensure that the index is within the bounds of the node.
    pub(super) unsafe fn new_at(node: &'a mut PageTableLock<E, C>, idx: usize) -> Self {
        // SAFETY: The index is within the bound.
        let pte = unsafe { node.read_pte(idx) };
        Self { pte, idx, node }
    }
}
