// SPDX-License-Identifier: MPL-2.0

//! This module specifies the type of the children of a page table node.

use core::mem::ManuallyDrop;

use ostd_pod::Pod;

use super::{PageTableNode, PageTableNodeRef, PteTrait};
use crate::{
    mm::{
        HasPaddr, PageTableFlags, PagingLevel,
        page_table::{PageTableConfig, PteScalar},
    },
    sync::RcuDrop,
};

/// A page table entry that owns the child of a page table node if present.
#[derive(Debug)]
pub(in crate::mm) enum PteState<C: PageTableConfig> {
    /// A child page table node.
    PageTable(RcuDrop<PageTableNode<C>>),
    /// A mapped item, often a mapped physical frame. The actual type is
    /// defined by the page table configuration.
    Mapped(RcuDrop<C::Item>),
    Absent,
}

impl<C: PageTableConfig> PteState<C> {
    /// Returns whether the child is not present.
    pub(in crate::mm) fn is_absent(&self) -> bool {
        matches!(self, PteState::Absent)
    }

    pub(super) fn into_pte(self) -> C::E {
        match self {
            PteState::PageTable(node) => {
                let paddr = node.paddr();
                let level = node.level();
                let _ = ManuallyDrop::new(node);
                C::E::from_repr(&PteScalar::PageTable(paddr, PageTableFlags::empty()), level)
            }
            PteState::Mapped(item) => {
                // SAFETY: The item will not be dropped when called with
                // `item_into_raw`. The resulting scalar entry, when converted
                // back to the item, will not be dropped before the RCU grace
                // period (see `PteState::from_pte`).
                let (item, panic_guard) = unsafe { RcuDrop::into_inner(item) };
                let (paddr, level, prop) = C::item_into_raw(item);
                panic_guard.forget();
                C::E::from_repr(&PteScalar::Mapped(paddr, prop), level)
            }
            PteState::Absent => C::E::new_zeroed(),
        }
    }

    /// # Safety
    ///
    /// The provided PTE must be the output of [`Self::into_pte`], and the PTE:
    ///  - must not be used to created a [`PteState`] twice;
    ///  - must not be referenced by a living [`PteStateRef`].
    ///
    /// The level must match the original level of the child.
    pub(super) unsafe fn from_pte(pte: C::E, level: PagingLevel) -> Self {
        let repr = pte.to_repr(level);

        match repr {
            PteScalar::Absent => PteState::Absent,
            PteScalar::PageTable(paddr, _) => {
                // SAFETY: The caller ensures that this node was created by
                // `into_pte`, so that restoring the forgotten reference is safe.
                let node = unsafe { PageTableNode::from_raw(paddr) };
                debug_assert_eq!(node.level(), level - 1);
                PteState::PageTable(RcuDrop::new(node))
            }
            PteScalar::Mapped(paddr, prop) => {
                // SAFETY: The caller ensures that this item was created by
                // `into_pte`, so that restoring the forgotten item is safe.
                let item = unsafe { C::item_from_raw(paddr, level, prop) };
                PteState::Mapped(RcuDrop::new(item))
            }
        }
    }
}

/// A reference to the child of a page table node.
#[derive(Debug)]
pub(in crate::mm) enum PteStateRef<'a, C: PageTableConfig> {
    /// A child page table node.
    PageTable(PageTableNodeRef<'a, C>),
    /// A reference to a mapped item, often a mapped physical frame. The actual
    /// reference type is defined by the page table configuration.
    Mapped(C::ItemRef<'a>),
    Absent,
}

impl<C: PageTableConfig> PteStateRef<'_, C> {
    /// # Safety
    ///
    /// The caller must ensure that:
    ///  - the PTE must be the output of a [`PteState::into_pte`], but the
    ///    accessed/dirty bits of the mapped page property can be different;
    ///  - the child, if exists, must outlive the created reference;
    ///  - the provided level must be the same with the level of the page table
    ///    node that contains this PTE.
    pub(super) unsafe fn from_pte(pte: &C::E, level: PagingLevel) -> Self {
        let repr = pte.to_repr(level);

        match repr {
            PteScalar::Absent => PteStateRef::Absent,
            PteScalar::PageTable(paddr, _) => {
                // SAFETY: The caller ensures that the lifetime of the child is
                // contained by the residing node, and the physical address is
                // valid since the entry is present.
                let node = unsafe { PageTableNodeRef::borrow_paddr(paddr) };
                debug_assert_eq!(node.level(), level - 1);
                PteStateRef::PageTable(node)
            }
            PteScalar::Mapped(paddr, prop) => {
                // SAFETY: The caller ensures that the lifetime of the item is
                // contained by the residing node, and the physical address is
                // valid since the entry is present.
                let item = unsafe { C::item_ref_from_raw(paddr, level, prop) };
                PteStateRef::Mapped(item)
            }
        }
    }
}
