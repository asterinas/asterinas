// SPDX-License-Identifier: MPL-2.0

//! This module provides accessors to the page table entries in a node.

use core::sync::atomic::Ordering;

use super::{Child, ChildRef, PageTableEntryTrait, PageTableGuard, PageTableNode};
use crate::{
    mm::{
        nr_subpage_per_huge,
        page_prop::PageProperty,
        page_size,
        page_table::{PageTableConfig, PageTableNodeRef},
        vm_space::Status,
    },
    sync::RcuDrop,
    task::atomic_mode::InAtomicMode,
};

/// A view of an entry in a page table node.
///
/// It can be borrowed from a node using the [`PageTableGuard::entry`] method.
///
/// This is a static reference to an entry in a node that does not account for
/// a dynamic reference count to the child. It can be used to create a owned
/// handle, which is a [`Child`].
#[derive(Debug)]
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
        !self.pte.is_present() && self.pte.paddr() == 0
    }

    /// Returns if the entry maps to a page table node.
    pub(in crate::mm) fn is_node(&self) -> bool {
        self.pte.is_present() && !self.pte.is_last(self.node.level())
    }

    /// Gets a reference to the child.
    pub(in crate::mm) fn to_ref(&self) -> ChildRef<'rcu, C> {
        // SAFETY:
        //  - The PTE outlives the reference (since we have `&self`).
        //  - The level matches the current node.
        unsafe { ChildRef::from_pte(&self.pte, self.node.level()) }
    }

    /// Operates on the mapping properties of the entry.
    ///
    /// It only modifies the properties if the entry is present.
    pub(in crate::mm) fn protect(
        &mut self,
        prot_op: &mut impl FnMut(&mut PageProperty),
        status_op: &mut impl FnMut(&mut Status),
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
                // Protect a status.

                // SAFETY: The physical address was written as a valid status.
                let mut status = unsafe { Status::from_raw_inner(paddr) };
                status_op(&mut status);
                self.pte.set_paddr(status.into_raw_inner());
            }
        }

        // SAFETY:
        //  1. The index is within the bounds.
        //  2. We replace the PTE with a new one, which differs only in
        //     `PageProperty`, so it's in `C` and at the correct paging level.
        //  3. The child is still owned by the page table node.
        unsafe { self.node.write_pte(self.idx, self.pte) };
    }

    /// Replaces the entry with a new child.
    ///
    /// The old child is returned.
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
            Child::None => {}
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

        self.pte = new_child.into_pte();

        // SAFETY:
        //  1. The index is within the bounds.
        //  2. The new PTE is a child in `C` and at the correct paging level.
        //  3. The ownership of the child is passed to the page table node.
        unsafe { self.node.write_pte(self.idx, self.pte) };

        old_child
    }

    /// Allocates a new child page table node and replaces the entry with it.
    ///
    /// If the old entry is not none, the operation will fail and return `None`.
    /// Otherwise, the lock guard of the new child page table node is returned.
    pub(in crate::mm::page_table) fn alloc_if_none(
        &mut self,
        guard: &'rcu dyn InAtomicMode,
    ) -> Option<PageTableGuard<'rcu, C>> {
        if !(self.is_none() && self.node.level() > 1) {
            return None;
        }

        let level = self.node.level();
        let new_page = RcuDrop::new(PageTableNode::<C>::alloc(level - 1));

        let paddr = new_page.start_paddr();
        // SAFETY: The page table won't be dropped before the RCU grace period
        // ends, so it outlives `'rcu`.
        let pt_ref = unsafe { PageTableNodeRef::borrow_paddr(paddr) };

        // Lock before writing the PTE, so no one else can operate on it.
        let pt_lock_guard = pt_ref.lock(guard);

        self.pte = Child::PageTable(new_page).into_pte();

        // SAFETY:
        //  1. The index is within the bounds.
        //  2. The new PTE is a child in `C` and at the correct paging level.
        //  3. The ownership of the child is passed to the page table node.
        core::sync::atomic::fence(Ordering::Release);
        unsafe { self.node.write_pte(self.idx, self.pte) };

        *self.node.nr_children_mut() += 1;

        Some(pt_lock_guard)
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
        guard: &'rcu dyn InAtomicMode,
    ) -> Option<PageTableGuard<'rcu, C>> {
        let level = self.node.level();

        let is_huge_page = self.pte.is_present() && self.pte.is_last(level) && level > 1;
        let is_huge_status = !self.pte.is_present() && level > 1 && self.pte.paddr() != 0;

        if !is_huge_page && !is_huge_status {
            return None;
        }

        let pa = self.pte.paddr();
        let prop = self.pte.prop();

        let new_page = if is_huge_status {
            let status = unsafe { Status::from_raw_inner(pa) };
            RcuDrop::new(PageTableNode::<C>::alloc_marked(level - 1, status))
        } else {
            debug_assert!(is_huge_page);
            RcuDrop::new(PageTableNode::<C>::alloc(level - 1))
        };

        let paddr = new_page.start_paddr();
        // SAFETY: The page table won't be dropped before the RCU grace period
        // ends, so it outlives `'rcu`.
        let pt_ref = unsafe { PageTableNodeRef::borrow_paddr(paddr) };

        // Lock before writing the PTE, so no one else can operate on it.
        let mut pt_lock_guard = pt_ref.lock(guard);

        if is_huge_page {
            debug_assert!(!is_huge_status);
            for i in 0..nr_subpage_per_huge::<C>() {
                let small_pa = pa + i * page_size::<C>(level - 1);
                let mut entry = pt_lock_guard.entry(i);
                let old = entry.replace(Child::Frame(small_pa, level - 1, prop));
                debug_assert!(old.is_none());
            }
        }

        self.pte = Child::PageTable(new_page).into_pte();

        // SAFETY:
        //  1. The index is within the bounds.
        //  2. The new PTE is a child in `C` and at the correct paging level.
        //  3. The ownership of the child is passed to the page table node.
        core::sync::atomic::fence(Ordering::Release);
        unsafe { self.node.write_pte(self.idx, self.pte) };

        Some(pt_lock_guard)
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
