// SPDX-License-Identifier: MPL-2.0

use alloc::{boxed::Box, sync::Arc};

use super::{nr_ptes_per_node, page_size, PageTableEntryTrait};
use crate::{
    sync::SpinLock,
    vm::{page_prop::PageProperty, Paddr, PagingConstsTrait, VmAllocOptions, VmFrame},
};

/// A page table frame.
/// It's also frequently referred to as a page table in many architectural documentations.
/// Cloning a page table frame will create a deep copy of the page table.
#[derive(Debug)]
pub(super) struct PageTableFrame<E: PageTableEntryTrait, C: PagingConstsTrait>
where
    [(); nr_ptes_per_node::<C>()]:,
    [(); C::NR_LEVELS]:,
{
    inner: VmFrame,
    /// TODO: all the following fields can be removed if frame metadata is introduced.
    /// Here we allow 2x space overhead each frame temporarily.
    #[allow(clippy::type_complexity)]
    children: Box<[Child<E, C>; nr_ptes_per_node::<C>()]>,
    nr_valid_children: usize,
}

pub(super) type PtfRef<E, C> = Arc<SpinLock<PageTableFrame<E, C>>>;

#[derive(Debug)]
pub(super) enum Child<E: PageTableEntryTrait, C: PagingConstsTrait>
where
    [(); nr_ptes_per_node::<C>()]:,
    [(); C::NR_LEVELS]:,
{
    PageTable(PtfRef<E, C>),
    Frame(VmFrame),
    /// Frames not tracked by the frame allocator.
    Untracked(Paddr),
    None,
}

impl<E: PageTableEntryTrait, C: PagingConstsTrait> Child<E, C>
where
    [(); nr_ptes_per_node::<C>()]:,
    [(); C::NR_LEVELS]:,
{
    pub(super) fn is_pt(&self) -> bool {
        matches!(self, Child::PageTable(_))
    }
    pub(super) fn is_frame(&self) -> bool {
        matches!(self, Child::Frame(_))
    }
    pub(super) fn is_none(&self) -> bool {
        matches!(self, Child::None)
    }
    pub(super) fn is_some(&self) -> bool {
        !self.is_none()
    }
    pub(super) fn is_untyped(&self) -> bool {
        matches!(self, Child::Untracked(_))
    }
    /// Is a last entry that maps to a physical address.
    pub(super) fn is_last(&self) -> bool {
        matches!(self, Child::Frame(_) | Child::Untracked(_))
    }
    fn paddr(&self) -> Option<Paddr> {
        match self {
            Child::PageTable(node) => {
                // Chance if dead lock is zero because it is only called by [`PageTableFrame::protect`],
                // and the cursor will not protect a node while holding the lock.
                Some(node.lock().start_paddr())
            }
            Child::Frame(frame) => Some(frame.start_paddr()),
            Child::Untracked(pa) => Some(*pa),
            Child::None => None,
        }
    }
}

impl<E: PageTableEntryTrait, C: PagingConstsTrait> Clone for Child<E, C>
where
    [(); nr_ptes_per_node::<C>()]:,
    [(); C::NR_LEVELS]:,
{
    /// This is a shallow copy.
    fn clone(&self) -> Self {
        match self {
            Child::PageTable(ptf) => Child::PageTable(ptf.clone()),
            Child::Frame(frame) => Child::Frame(frame.clone()),
            Child::Untracked(pa) => Child::Untracked(*pa),
            Child::None => Child::None,
        }
    }
}

impl<E: PageTableEntryTrait, C: PagingConstsTrait> PageTableFrame<E, C>
where
    [(); nr_ptes_per_node::<C>()]:,
    [(); C::NR_LEVELS]:,
{
    pub(super) fn new() -> Self {
        Self {
            inner: VmAllocOptions::new(1).alloc_single().unwrap(),
            children: Box::new(core::array::from_fn(|_| Child::None)),
            nr_valid_children: 0,
        }
    }

    pub(super) fn start_paddr(&self) -> Paddr {
        self.inner.start_paddr()
    }

    pub(super) fn child(&self, idx: usize) -> &Child<E, C> {
        debug_assert!(idx < nr_ptes_per_node::<C>());
        &self.children[idx]
    }

    /// The number of mapped frames or page tables.
    /// This is to track if we can free itself.
    pub(super) fn nr_valid_children(&self) -> usize {
        self.nr_valid_children
    }

    /// Read the info from a page table entry at a given index.
    pub(super) fn read_pte_prop(&self, idx: usize) -> PageProperty {
        self.read_pte(idx).prop()
    }

    /// Split the untracked huge page mapped at `idx` to smaller pages.
    pub(super) fn split_untracked_huge(&mut self, cur_level: usize, idx: usize) {
        debug_assert!(idx < nr_ptes_per_node::<C>());
        debug_assert!(cur_level > 1);
        let Child::Untracked(pa) = self.children[idx] else {
            panic!("split_untracked_huge: not an untyped huge page");
        };
        let prop = self.read_pte_prop(idx);
        let mut new_frame = Self::new();
        for i in 0..nr_ptes_per_node::<C>() {
            let small_pa = pa + i * page_size::<C>(cur_level - 1);
            new_frame.set_child(i, Child::Untracked(small_pa), Some(prop), cur_level - 1 > 1);
        }
        self.set_child(
            idx,
            Child::PageTable(Arc::new(SpinLock::new(new_frame))),
            Some(prop),
            false,
        );
    }

    /// Map a child at a given index.
    /// If mapping a non-none child, please give the property to map the child.
    pub(super) fn set_child(
        &mut self,
        idx: usize,
        child: Child<E, C>,
        prop: Option<PageProperty>,
        huge: bool,
    ) {
        assert!(idx < nr_ptes_per_node::<C>());
        // SAFETY: the index is within the bound and the PTE to be written is valid.
        // And the physical address of PTE points to initialized memory.
        // This applies to all the following `write_pte` invocations.
        unsafe {
            match &child {
                Child::PageTable(node) => {
                    debug_assert!(!huge);
                    let frame = node.lock();
                    self.write_pte(
                        idx,
                        E::new(frame.inner.start_paddr(), prop.unwrap(), false, false),
                    );
                    self.nr_valid_children += 1;
                }
                Child::Frame(frame) => {
                    debug_assert!(!huge); // `VmFrame` currently can only be a regular page.
                    self.write_pte(idx, E::new(frame.start_paddr(), prop.unwrap(), false, true));
                    self.nr_valid_children += 1;
                }
                Child::Untracked(pa) => {
                    self.write_pte(idx, E::new(*pa, prop.unwrap(), huge, true));
                    self.nr_valid_children += 1;
                }
                Child::None => {
                    self.write_pte(idx, E::new_absent());
                }
            }
        }
        if self.children[idx].is_some() {
            self.nr_valid_children -= 1;
        }
        self.children[idx] = child;
    }

    /// Protect an already mapped child at a given index.
    pub(super) fn protect(&mut self, idx: usize, prop: PageProperty, level: usize) {
        debug_assert!(self.children[idx].is_some());
        let paddr = self.children[idx].paddr().unwrap();
        // SAFETY: the index is within the bound and the PTE is valid.
        unsafe {
            self.write_pte(
                idx,
                E::new(paddr, prop, level > 1, self.children[idx].is_last()),
            );
        }
    }

    fn read_pte(&self, idx: usize) -> E {
        assert!(idx < nr_ptes_per_node::<C>());
        // SAFETY: the index is within the bound and PTE is plain-old-data.
        unsafe { (self.inner.as_ptr() as *const E).add(idx).read() }
    }

    /// Write a page table entry at a given index.
    ///
    /// # Safety
    ///
    /// The caller must ensure that:
    ///  - the index is within bounds;
    ///  - the PTE is valid an the physical address in the PTE points to initialized memory.
    unsafe fn write_pte(&mut self, idx: usize, pte: E) {
        (self.inner.as_mut_ptr() as *mut E).add(idx).write(pte);
    }
}

impl<E: PageTableEntryTrait, C: PagingConstsTrait> Clone for PageTableFrame<E, C>
where
    [(); nr_ptes_per_node::<C>()]:,
    [(); C::NR_LEVELS]:,
{
    /// Make a deep copy of the page table.
    /// The child page tables are also being deep copied.
    fn clone(&self) -> Self {
        let new_frame = VmAllocOptions::new(1).alloc_single().unwrap();
        let new_ptr = new_frame.as_mut_ptr() as *mut E;
        let children = Box::new(core::array::from_fn(|i| match self.child(i) {
            Child::PageTable(node) => unsafe {
                let frame = node.lock();
                // Possibly a cursor is waiting for the root lock to recycle this node.
                // We can skip copying empty page table nodes.
                if frame.nr_valid_children() != 0 {
                    let cloned = frame.clone();
                    let pte = self.read_pte(i);
                    new_ptr.add(i).write(E::new(
                        cloned.inner.start_paddr(),
                        pte.prop(),
                        false,
                        false,
                    ));
                    Child::PageTable(Arc::new(SpinLock::new(cloned)))
                } else {
                    Child::None
                }
            },
            Child::Frame(_) | Child::Untracked(_) => {
                unsafe {
                    new_ptr.add(i).write(self.read_pte(i));
                }
                self.children[i].clone()
            }
            Child::None => Child::None,
        }));
        Self {
            inner: new_frame,
            children,
            nr_valid_children: self.nr_valid_children,
        }
    }
}
