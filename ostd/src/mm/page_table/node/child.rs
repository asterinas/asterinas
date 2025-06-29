// SPDX-License-Identifier: MPL-2.0

//! This module specifies the type of the children of a page table node.

use core::mem::ManuallyDrop;

use super::{PageTableEntryTrait, PageTableNode, PageTableNodeRef};
use crate::mm::{page_prop::PageProperty, page_table::PageTableConfig, Paddr, PagingLevel};

/// A page table entry that owns the child of a page table node if present.
#[derive(Debug)]
pub(in crate::mm) enum Child<C: PageTableConfig> {
    /// A child page table node.
    PageTable(PageTableNode<C>),
    /// Physical address of a mapped physical frame.
    ///
    /// It is associated with the virtual page property and the level of the
    /// mapping node, which decides the size of the frame.
    Frame(Paddr, PagingLevel, PageProperty),
    None,
}

impl<C: PageTableConfig> Child<C> {
    /// Returns whether the child is not present.
    pub(in crate::mm) fn is_none(&self) -> bool {
        matches!(self, Child::None)
    }

    pub(super) fn into_pte(self) -> C::E {
        match self {
            Child::PageTable(node) => {
                let paddr = node.start_paddr();
                let _ = ManuallyDrop::new(node);
                C::E::new_pt(paddr)
            }
            Child::Frame(paddr, level, prop) => C::E::new_page(paddr, level, prop),
            Child::None => C::E::new_absent(),
        }
    }

    /// # Safety
    ///
    /// The provided PTE must be the output of [`Self::into_pte`], and the PTE:
    ///  - must not be used to created a [`Child`] twice;
    ///  - must not be referenced by a living [`ChildRef`].
    ///
    /// The level must match the original level of the child.
    pub(super) unsafe fn from_pte(pte: C::E, level: PagingLevel) -> Self {
        let paddr = pte.paddr();
        if !pte.is_present() && paddr == 0 {
            return Child::None;
        }

        if pte.is_present() && !pte.is_last(level) {
            // SAFETY: The caller ensures that this node was created by
            // `into_pte`, so that restoring the forgotten reference is safe.
            let node = unsafe { PageTableNode::from_raw(paddr) };
            debug_assert_eq!(node.level(), level - 1);
            return Child::PageTable(node);
        }

        Child::Frame(paddr, level, pte.prop())
    }
}

/// A reference to the child of a page table node.
#[derive(Debug)]
pub(in crate::mm) enum ChildRef<'a, C: PageTableConfig> {
    /// A child page table node.
    PageTable(PageTableNodeRef<'a, C>),
    /// Physical address of a mapped physical frame.
    ///
    /// It is associated with the virtual page property and the level of the
    /// mapping node, which decides the size of the frame.
    Frame(Paddr, PagingLevel, PageProperty),
    None,
}

impl<C: PageTableConfig> ChildRef<'_, C> {
    /// Converts a PTE to a child.
    ///
    /// # Safety
    ///
    /// The PTE must be the output of a [`Child::into_pte`], where the child
    /// outlives the reference created by this function.
    ///
    /// The provided level must be the same with the level of the page table
    /// node that contains this PTE.
    pub(super) unsafe fn from_pte(pte: &C::E, level: PagingLevel) -> Self {
        let paddr = pte.paddr();
        if !pte.is_present() && paddr == 0 {
            return ChildRef::None;
        }

        if pte.is_present() && !pte.is_last(level) {
            // SAFETY: The caller ensures that the lifetime of the child is
            // contained by the residing node, and the physical address is
            // valid since the entry is present.
            let node = unsafe { PageTableNodeRef::borrow_paddr(paddr) };
            debug_assert_eq!(node.level(), level - 1);
            return ChildRef::PageTable(node);
        }

        ChildRef::Frame(paddr, level, pte.prop())
    }
}
