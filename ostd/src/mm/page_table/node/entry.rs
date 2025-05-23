// SPDX-License-Identifier: MPL-2.0

//! This module provides accessors to the page table entries in a node.

use core::mem::ManuallyDrop;

use super::{Child, ChildRef, PageTableEntryTrait, PageTableGuard, PageTableNode};
use crate::{
    mm::{
        nr_subpage_per_huge,
        page_prop::PageProperty,
        page_size,
        page_table::{PageTableConfig, PageTableNodeRef},
    },
    sync::RcuDrop,
};

/// A view of an entry in a page table node.
///
/// It can be borrowed from a node using the [`PageTableGuard::entry`] method.
///
/// This is a static reference to an entry in a node that does not account for
/// a dynamic reference count to the child. It can be used to create a owned
/// handle, which is a [`Child`].
// Note that
pub(in crate::mm) struct Entry<'a, 'rcu, C: PageTableConfig> {
    /// The page table entry.
    ///
    /// We store the page table entry here to optimize the number of reads from
    /// the node. We cannot hold a `&mut E` reference to the entry because that
    /// other CPUs may modify the memory location for accessed/dirty bits. Such
    /// accesses will violate the aliasing rules of Rust and cause undefined
    /// behaviors.
    pte: C::E,
    /// The index of the entry in the node.
    idx: usize,
    /// The node that contains the entry.
    node: &'a mut PageTableGuard<'rcu, C>,
}

impl<'a, 'rcu, C: PageTableConfig> Entry<'a, 'rcu, C> {
    /// Returns if the entry does not map to anything.
    pub(in crate::mm) fn is_none(&self) -> bool {
        !self.pte.is_present()
    }

    /// Returns if the entry maps to a page table node.
    pub(in crate::mm) fn is_node(&self) -> bool {
        self.pte.is_present() && !self.pte.is_last(self.node.level())
    }

    /// Gets a reference to the child.
    pub(in crate::mm) fn to_ref(&self) -> ChildRef<'rcu, C> {
        // SAFETY: The entry structure represents an existent entry with the
        // right node information.
        unsafe { ChildRef::from_pte(&self.pte, self.node.level()) }
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
        //     `PageProperty`, so the level still matches the current
        //     page table node.
        unsafe { self.node.write_pte(self.idx, self.pte) };
    }

    /// Replaces the entry with a new child.
    ///
    /// The old child is returned.
    ///
    /// # Safety
    ///
    /// The caller must ensure that:
    ///  - if the new child is `PageTable`, the address must point to a valid
    ///    page table node, the level must be correct and the pointed node must
    ///    outlive this node;
    ///  - if the new child is `Frame`, the new established mapping must be
    ///    a valid mapping (TODO: specify valid mappings).
    ///
    /// # Panics
    ///
    /// The method panics if the level of the new child does not match the
    /// current node.
    pub(in crate::mm) fn replace(&mut self, new_child: Child<C>) -> Child<C> {
        match &new_child {
            Child::PageTable(node) => {
                assert_eq!(node.level(), self.node.level() - 1);
            }
            Child::Frame(_, level, _) => {
                assert_eq!(*level, self.node.level());
            }
            _ => {}
        }

        // SAFETY:
        //  - The PTE is not referenced by other `ChildRef`s (since we have `&mut self`).
        //  - The level matches the current node.
        let old_child = unsafe { Child::from_pte(self.pte, self.node.level()) };

        if old_child.is_none() && !new_child.is_none() {
            *self.node.nr_children_mut() += 1;
        } else if !old_child.is_none() && new_child.is_none() {
            *self.node.nr_children_mut() -= 1;
        }

        let new_pte = new_child.into_pte();

        // SAFETY:
        //  1. The index is within the bounds.
        //  2. The new PTE is a valid child whose level matches the current page table node.
        unsafe { self.node.write_pte(self.idx, new_pte) };

        self.pte = new_pte;

        old_child
    }

    /// Allocates a new child page table node and replaces the entry with it.
    ///
    /// If the old entry is not none, the operation will fail and return `None`.
    /// Otherwise, the lock guard of the new child page table node is returned.
    pub(in crate::mm::page_table) fn alloc_if_none(&mut self) -> Option<PageTableGuard<'rcu, C>> {
        if !self.is_none() {
            return None;
        }

        let level = self.node.level();
        let new_page = PageTableNode::<C>::alloc(level - 1);

        let paddr = new_page.start_paddr();
        let _ = ManuallyDrop::new(new_page.lock());

        // SAFETY:
        //  1. The index is within the bounds.
        //  2. The new PTE is a valid child whose level matches the current page table node.
        unsafe {
            self.node.write_pte(
                self.idx,
                Child::PageTable(RcuDrop::new(new_page)).into_pte(),
            )
        };

        *self.node.nr_children_mut() += 1;

        // SAFETY: The page table won't be dropped before the RCU grace period
        // ends, so it outlives `'rcu`.
        let pt_ref = unsafe { PageTableNodeRef::borrow_paddr(paddr) };
        // SAFETY: The node is locked and there are no other guards.
        Some(unsafe { pt_ref.make_guard_unchecked() })
    }

    /// Splits the entry to smaller pages if it maps to a huge page.
    ///
    /// If the entry does map to a huge page, it is split into smaller pages
    /// mapped by a child page table node. The new child page table node
    /// is returned.
    ///
    /// If the entry does not map to a untracked huge page, the method returns
    /// `None`.
    pub(in crate::mm::page_table) fn split_if_mapped_huge(
        &mut self,
    ) -> Option<PageTableGuard<'rcu, C>> {
        let level = self.node.level();

        if !(self.pte.is_last(level) && level > 1) {
            return None;
        }

        let pa = self.pte.paddr();
        let prop = self.pte.prop();

        let new_page = PageTableNode::<C>::alloc(level - 1);
        let mut guard = new_page.lock();

        for i in 0..nr_subpage_per_huge::<C>() {
            let small_pa = pa + i * page_size::<C>(level - 1);
            let mut entry = guard.entry(i);
            let old = entry.replace(Child::Frame(small_pa, level - 1, prop));
            debug_assert!(old.is_none());
        }

        let paddr = new_page.start_paddr();
        let _ = ManuallyDrop::new(guard);

        // SAFETY:
        //  1. The index is within the bounds.
        //  2. The new PTE is a valid child whose level matches the current page table node.
        unsafe {
            self.node.write_pte(
                self.idx,
                Child::PageTable(RcuDrop::new(new_page)).into_pte(),
            )
        };

        // SAFETY: The page table won't be dropped before the RCU grace period
        // ends, so it outlives `'rcu`.
        let pt_ref = unsafe { PageTableNodeRef::borrow_paddr(paddr) };
        // SAFETY: The node is locked and there are no other guards.
        Some(unsafe { pt_ref.make_guard_unchecked() })
    }

    /// Create a new entry at the node with guard.
    ///
    /// # Safety
    ///
    /// The caller must ensure that the index is within the bounds of the node.
    pub(super) unsafe fn new_at(guard: &'a mut PageTableGuard<'rcu, C>, idx: usize) -> Self {
        // SAFETY: The index is within the bound.
        let pte = unsafe { guard.read_pte(idx) };
        Self {
            pte,
            idx,
            node: guard,
        }
    }
}
