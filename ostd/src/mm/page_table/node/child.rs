// SPDX-License-Identifier: MPL-2.0

//! This module specifies the type of the children of a page table node.

use core::{mem::ManuallyDrop, panic};

use super::{PageTableEntryTrait, RawPageTableNode};
use crate::{
    arch::mm::{PageTableEntry, PagingConsts},
    mm::{
        page::{
            meta::{MapTrackingStatus, PageTablePageMeta},
            DynPage, Page,
        },
        page_prop::PageProperty,
        Paddr, PagingConstsTrait, PagingLevel,
    },
};

/// A child of a page table node.
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
    /// provided information (level and tracking status) must align with the
    /// lost information during the conversion.
    ///
    /// This method should be only used no more than once for a PTE that has
    /// been converted from a child using the [`Child::into_pte`] method.
    pub(super) unsafe fn from_pte(
        pte: E,
        level: PagingLevel,
        is_tracked: MapTrackingStatus,
    ) -> Self {
        if !pte.is_present() {
            Child::None
        } else {
            let paddr = pte.paddr();
            if !pte.is_last(level) {
                Child::PageTable(RawPageTableNode::from_paddr(paddr))
            } else {
                match is_tracked {
                    MapTrackingStatus::Tracked => Child::Page(DynPage::from_raw(paddr), pte.prop()),
                    MapTrackingStatus::Untracked => Child::Untracked(paddr, level, pte.prop()),
                    MapTrackingStatus::NotApplicable => panic!("Invalid tracking status"),
                }
            }
        }
    }

    /// Gains an extra owning reference to the child.
    ///
    /// # Safety
    ///
    /// The provided PTE must be originated from [`Child::into_pte`]. And the
    /// provided information (level and tracking status) must align with the
    /// lost information during the conversion.
    ///
    /// This method must not be used with a PTE that has been restored to a
    /// child using the [`Child::from_pte`] method.
    pub(super) unsafe fn clone_from_pte(
        pte: &E,
        level: PagingLevel,
        is_tracked: MapTrackingStatus,
    ) -> Self {
        if !pte.is_present() {
            Child::None
        } else {
            let paddr = pte.paddr();
            if !pte.is_last(level) {
                Page::<PageTablePageMeta<E, C>>::inc_ref_count(paddr);
                Child::PageTable(RawPageTableNode::from_paddr(paddr))
            } else {
                match is_tracked {
                    MapTrackingStatus::Tracked => {
                        DynPage::inc_ref_count(paddr);
                        Child::Page(DynPage::from_raw(paddr), pte.prop())
                    }
                    MapTrackingStatus::Untracked => Child::Untracked(paddr, level, pte.prop()),
                    MapTrackingStatus::NotApplicable => panic!("Invalid tracking status"),
                }
            }
        }
    }
}
