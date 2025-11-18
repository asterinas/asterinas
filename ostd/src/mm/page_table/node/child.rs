// SPDX-License-Identifier: MPL-2.0

//! This module specifies the type of the children of a page table node.

use core::mem::ManuallyDrop;

use ostd_pod::Pod;

use super::{PageTableNode, PageTableNodeRef, PteTrait};
use crate::{
    mm::{
        HasPaddr, Paddr, PageTableFlags, PagingLevel,
        page_prop::PageProperty,
        page_table::{PageTableConfig, PteScalar},
    },
    sync::RcuDrop,
};

/// A page table entry that owns the child of a page table node if present.
#[derive(Debug)]
pub(in crate::mm) enum Child<C: PageTableConfig> {
    /// A child page table node.
    PageTable(RcuDrop<PageTableNode<C>>),
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
                let paddr = node.paddr();
                let level = node.level();
                let _ = ManuallyDrop::new(node);
                C::E::from_repr(&PteScalar::PageTable(paddr, PageTableFlags::empty()), level)
            }
            Child::Frame(paddr, level, prop) => {
                C::E::from_repr(&PteScalar::Mapped(paddr, prop), level)
            }
            Child::None => C::E::new_zeroed(),
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
        let repr = pte.to_repr(level);

        match repr {
            PteScalar::Absent => Child::None,
            PteScalar::PageTable(paddr, _) => {
                // SAFETY: The caller ensures that this node was created by
                // `into_pte`, so that restoring the forgotten reference is safe.
                let node = unsafe { PageTableNode::from_raw(paddr) };
                debug_assert_eq!(node.level(), level - 1);
                Child::PageTable(RcuDrop::new(node))
            }
            PteScalar::Mapped(paddr, prop) => Child::Frame(paddr, level, prop),
        }
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
        let repr = pte.to_repr(level);

        match repr {
            PteScalar::Absent => ChildRef::None,
            PteScalar::PageTable(paddr, _) => {
                // SAFETY: The caller ensures that the lifetime of the child is
                // contained by the residing node, and the physical address is
                // valid since the entry is present.
                let node = unsafe { PageTableNodeRef::borrow_paddr(paddr) };
                debug_assert_eq!(node.level(), level - 1);
                ChildRef::PageTable(node)
            }
            PteScalar::Mapped(paddr, prop) => ChildRef::Frame(paddr, level, prop),
        }
    }
}
