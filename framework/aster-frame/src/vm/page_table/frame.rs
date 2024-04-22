// SPDX-License-Identifier: MPL-2.0

use alloc::{boxed::Box, sync::Arc};

use super::{PageTableConstsTrait, PageTableEntryTrait};
use crate::{
    sync::SpinLock,
    vm::{VmAllocOptions, VmFrame},
};

/// A page table frame.
/// It's also frequently referred to as a page table in many architectural documentations.
/// Cloning a page table frame will create a deep copy of the page table.
#[derive(Debug)]
pub(super) struct PageTableFrame<E: PageTableEntryTrait, C: PageTableConstsTrait>
where
    [(); C::NR_ENTRIES_PER_FRAME]:,
    [(); C::NR_LEVELS]:,
{
    pub inner: VmFrame,
    /// TODO: all the following fields can be removed if frame metadata is introduced.
    /// Here we allow 2x space overhead each frame temporarily.
    #[allow(clippy::type_complexity)]
    pub child: Box<[Option<Child<E, C>>; C::NR_ENTRIES_PER_FRAME]>,
    /// The number of mapped frames or page tables.
    /// This is to track if we can free itself.
    pub map_count: usize,
}

pub(super) type PtfRef<E, C> = Arc<SpinLock<PageTableFrame<E, C>>>;

#[derive(Debug)]
pub(super) enum Child<E: PageTableEntryTrait, C: PageTableConstsTrait>
where
    [(); C::NR_ENTRIES_PER_FRAME]:,
    [(); C::NR_LEVELS]:,
{
    PageTable(PtfRef<E, C>),
    Frame(VmFrame),
}

impl<E: PageTableEntryTrait, C: PageTableConstsTrait> Clone for Child<E, C>
where
    [(); C::NR_ENTRIES_PER_FRAME]:,
    [(); C::NR_LEVELS]:,
{
    /// This is a shallow copy.
    fn clone(&self) -> Self {
        match self {
            Child::PageTable(ptf) => Child::PageTable(ptf.clone()),
            Child::Frame(frame) => Child::Frame(frame.clone()),
        }
    }
}

impl<E: PageTableEntryTrait, C: PageTableConstsTrait> PageTableFrame<E, C>
where
    [(); C::NR_ENTRIES_PER_FRAME]:,
    [(); C::NR_LEVELS]:,
{
    pub(super) fn new() -> Self {
        Self {
            inner: VmAllocOptions::new(1).alloc_single().unwrap(),
            child: Box::new(core::array::from_fn(|_| None)),
            map_count: 0,
        }
    }
}

impl<E: PageTableEntryTrait, C: PageTableConstsTrait> Clone for PageTableFrame<E, C>
where
    [(); C::NR_ENTRIES_PER_FRAME]:,
    [(); C::NR_LEVELS]:,
{
    /// Make a deep copy of the page table.
    /// The child page tables are also being deep copied.
    fn clone(&self) -> Self {
        let new_frame = VmAllocOptions::new(1).alloc_single().unwrap();
        let new_ptr = new_frame.as_mut_ptr() as *mut E;
        let ptr = self.inner.as_ptr() as *const E;
        let child = Box::new(core::array::from_fn(|i| {
            self.child[i].as_ref().map(|child| match child {
                Child::PageTable(ptf) => unsafe {
                    let frame = ptf.lock();
                    let cloned = frame.clone();
                    let pte = ptr.add(i).read();
                    new_ptr.add(i).write(E::new(
                        cloned.inner.start_paddr(),
                        pte.info().prop,
                        false,
                        false,
                    ));
                    Child::PageTable(Arc::new(SpinLock::new(cloned)))
                },
                Child::Frame(frame) => {
                    unsafe {
                        let pte = ptr.add(i).read();
                        new_ptr.add(i).write(pte);
                    }
                    Child::Frame(frame.clone())
                }
            })
        }));
        Self {
            inner: new_frame,
            child,
            map_count: self.map_count,
        }
    }
}
