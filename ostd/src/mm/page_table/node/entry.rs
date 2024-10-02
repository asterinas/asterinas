// SPDX-License-Identifier: MPL-2.0

//! This module provides accessors to the page table entries in a node.

use super::{Child, PageTableEntryTrait, PageTableNode};
use crate::mm::{
    nr_subpage_per_huge, page::meta::MapTrackingStatus, page_prop::PageProperty, page_size,
    PagingConstsTrait,
};

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

    /// Returns if the entry maps to a page.
    pub(in crate::mm) fn is_last(&self) -> bool {
        self.pte.is_present() && self.pte.is_last(self.node.level())
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
    pub(in crate::mm) fn protect(&mut self, op: &mut impl FnMut(&mut PageProperty)) {
        let prop = self.pte.prop();
        let mut new_prop = prop;
        op(&mut new_prop);
        if prop != new_prop {
            self.pte.set_prop(new_prop);
            self.node.write_pte(self.idx, self.pte);
        }
    }

    /// Replaces the entry with a new child.
    ///
    /// The old child is returned.
    ///
    /// The provided child must match with the level of the node.
    pub(in crate::mm) fn replace(self, new_child: Child<E, C>) -> Child<E, C> {
        // It should be ensured by the cursor.
        #[cfg(debug_assertions)]
        match &new_child {
            Child::PageTable(_) => {
                debug_assert!(self.node.level() > 1);
            }
            Child::Page(p, _) => {
                debug_assert!(self.node.level() == p.level());
                debug_assert!(self.node.is_tracked() == MapTrackingStatus::Tracked);
            }
            Child::Untracked(_, level, _) => {
                debug_assert!(self.node.level() == *level);
                debug_assert!(self.node.is_tracked() == MapTrackingStatus::Untracked);
            }
            Child::None => {}
        }

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

        self.node.write_pte(self.idx, new_child.into_pte());

        old_child
    }

    /// Splits the untracked huge page to smaller pages.
    ///
    /// The new child page table node is returned.
    ///
    /// This entry should be an untracked huge page.
    pub(in crate::mm) fn split_untracked_huge(self) -> PageTableNode<E, C> {
        // These should be ensured by the cursor.
        debug_assert!(self.node.level() > 1);

        let pa = self.pte.paddr();
        let level = self.node.level();
        let prop = self.pte.prop();

        let mut new_page = PageTableNode::<E, C>::alloc(level - 1, MapTrackingStatus::Untracked);
        for i in 0..nr_subpage_per_huge::<C>() {
            let small_pa = pa + i * page_size::<C>(level - 1);
            let _ = new_page
                .entry(i)
                .replace(Child::Untracked(small_pa, level - 1, prop));
        }

        let _ = self.replace(Child::PageTable(new_page.clone_raw()));

        new_page
    }

    pub(super) fn new_at(node: &'a mut PageTableNode<E, C>, idx: usize) -> Self {
        debug_assert!(idx < nr_subpage_per_huge::<C>());
        Self {
            pte: node.read_pte(idx),
            idx,
            node,
        }
    }
}
