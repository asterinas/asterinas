// SPDX-License-Identifier: MPL-2.0

//! This module specifies the type of the children of a page table node.

use core::{mem::ManuallyDrop, panic};

use super::{PageTableEntryTrait, RawPageTableNode};
use crate::{
    arch::mm::{PageTableEntry, PagingConsts},
    mm::{
        page::{inc_page_ref_count, meta::MapTrackingStatus, DynPage},
        page_prop::PageProperty,
        Paddr, PagingConstsTrait, PagingLevel,
    },
};

/// A child of a page table node.
///
/// This is a owning handle to a child of a page table node. If the child is
/// either a page table node or a page, it holds a reference count to the
/// corresponding page.
#[derive(Debug)]
pub(in crate::mm) enum Child<
    E: PageTableEntryTrait = PageTableEntry,
    C: PagingConstsTrait = PagingConsts,
> where
    [(); C::NR_LEVELS as usize]:,
{
    PageTable(RawPageTableNode<E, C>),
    Page(DynPage, PageProperty),
    /// Pages not tracked by handles.
    Untracked(Paddr, PagingLevel, PageProperty),
    None,
}

impl<E: PageTableEntryTrait, C: PagingConstsTrait> Child<E, C>
where
    [(); C::NR_LEVELS as usize]:,
{
    /// Returns whether the child does not map to anything.
    pub(in crate::mm) fn is_none(&self) -> bool {
        matches!(self, Child::None)
    }

    /// Returns whether the child is compatible with the given node.
    ///
    /// In other words, it checks whether the child can be a child of a node
    /// with the given level and tracking status.
    pub(super) fn is_compatible(
        &self,
        node_level: PagingLevel,
        is_tracked: MapTrackingStatus,
    ) -> bool {
        match self {
            Child::PageTable(pt) => node_level == pt.level() + 1,
            Child::Page(p, _) => {
                node_level == p.level() && is_tracked == MapTrackingStatus::Tracked
            }
            Child::Untracked(_, level, _) => {
                node_level == *level && is_tracked == MapTrackingStatus::Untracked
            }
            Child::None => true,
        }
    }

    /// Converts a child into a owning PTE.
    ///
    /// By conversion it loses information about whether the page is tracked
    /// or not. Also it loses the level information. However, the returned PTE
    /// takes the ownership (reference count) of the child.
    ///
    /// Usually this is for recording the PTE into a page table node. When the
    /// child is needed again by reading the PTE of a page table node, extra
    /// information should be provided using the [`Child::from_pte`] method.
    pub(super) fn into_pte(self) -> E {
        match self {
            Child::PageTable(pt) => {
                let pt = ManuallyDrop::new(pt);
                E::new_pt(pt.paddr())
            }
            Child::Page(page, prop) => {
                let level = page.level();
                E::new_page(page.into_raw(), level, prop)
            }
            Child::Untracked(pa, level, prop) => E::new_page(pa, level, prop),
            Child::None => E::new_absent(),
        }
    }

    /// Converts a PTE back to a child.
    ///
    /// # Safety
    ///
    /// The provided PTE must be originated from [`Child::into_pte`]. And the
    /// provided information (level and tracking status) must be the same with
    /// the lost information during the conversion. Strictly speaking, the
    /// provided arguments must be compatible with the original child (
    /// specified by [`Child::is_compatible`]).
    ///
    /// This method should be only used no more than once for a PTE that has
    /// been converted from a child using the [`Child::into_pte`] method.
    pub(super) unsafe fn from_pte(
        pte: E,
        level: PagingLevel,
        is_tracked: MapTrackingStatus,
    ) -> Self {
        if !pte.is_present() {
            return Child::None;
        }

        let paddr = pte.paddr();

        if !pte.is_last(level) {
            // SAFETY: The physical address points to a valid page table node
            // at the given level.
            return Child::PageTable(unsafe { RawPageTableNode::from_raw_parts(paddr, level - 1) });
        }

        match is_tracked {
            MapTrackingStatus::Tracked => {
                // SAFETY: The physical address points to a valid page.
                let page = unsafe { DynPage::from_raw(paddr) };
                Child::Page(page, pte.prop())
            }
            MapTrackingStatus::Untracked => Child::Untracked(paddr, level, pte.prop()),
            MapTrackingStatus::NotApplicable => panic!("Invalid tracking status"),
        }
    }

    /// Gains an extra owning reference to the child.
    ///
    /// # Safety
    ///
    /// The provided PTE must be originated from [`Child::into_pte`], which is
    /// the same requirement as the [`Child::from_pte`] method.
    ///
    /// This method must not be used with a PTE that has been restored to a
    /// child using the [`Child::from_pte`] method.
    pub(super) unsafe fn clone_from_pte(
        pte: &E,
        level: PagingLevel,
        is_tracked: MapTrackingStatus,
    ) -> Self {
        if !pte.is_present() {
            return Child::None;
        }

        let paddr = pte.paddr();

        if !pte.is_last(level) {
            // SAFETY: The physical address is valid and the PTE already owns
            // the reference to the page.
            unsafe { inc_page_ref_count(paddr) };
            // SAFETY: The physical address points to a valid page table node
            // at the given level.
            return Child::PageTable(unsafe { RawPageTableNode::from_raw_parts(paddr, level - 1) });
        }

        match is_tracked {
            MapTrackingStatus::Tracked => {
                // SAFETY: The physical address is valid and the PTE already owns
                // the reference to the page.
                unsafe { inc_page_ref_count(paddr) };
                // SAFETY: The physical address points to a valid page.
                let page = unsafe { DynPage::from_raw(paddr) };
                Child::Page(page, pte.prop())
            }
            MapTrackingStatus::Untracked => Child::Untracked(paddr, level, pte.prop()),
            MapTrackingStatus::NotApplicable => panic!("Invalid tracking status"),
        }
    }
}
