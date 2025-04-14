// SPDX-License-Identifier: MPL-2.0

//! This module defines page table node abstractions and the handle.
//!
//! The page table node is also frequently referred to as a page table in many architectural
//! documentations. It is essentially a page that contains page table entries (PTEs) that map
//! to child page tables nodes or mapped pages.
//!
//! This module leverages the page metadata to manage the page table pages, which makes it
//! easier to provide the following guarantees:
//!
//! The page table node is not freed when it is still in use by:
//!    - a parent page table node,
//!    - or a handle to a page table node,
//!    - or a processor.
//!
//! This is implemented by using a reference counter in the page metadata. If the above
//! conditions are not met, the page table node is ensured to be freed upon dropping the last
//! reference.
//!
//! One can acquire exclusive access to a page table node using merely the physical address of
//! the page table node. This is implemented by a lock in the page metadata. Here the
//! exclusiveness is only ensured for kernel code, and the processor's MMU is able to access the
//! page table node while a lock is held. So the modification to the PTEs should be done after
//! the initialization of the entity that the PTE points to. This is taken care in this module.
//!

mod child;
mod entry;

use core::{
    cell::SyncUnsafeCell,
    marker::PhantomData,
    mem::ManuallyDrop,
    sync::atomic::{AtomicU8, Ordering},
};

pub(in crate::mm) use self::{child::Child, entry::Entry};
use super::{nr_subpage_per_huge, PageTableEntryTrait};
use crate::{
    arch::mm::{PageTableEntry, PagingConsts},
    mm::{
        frame::{inc_frame_ref_count, meta::AnyFrameMeta, Frame},
        paddr_to_vaddr,
        page_table::{load_pte, store_pte},
        FrameAllocOptions, Infallible, Paddr, PagingConstsTrait, PagingLevel, VmReader,
    },
};

/// The raw handle to a page table node.
///
/// This handle is a referencer of a page table node. Thus creating and dropping it will affect
/// the reference count of the page table node. If dropped the raw handle as the last reference,
/// the page table node and subsequent children will be freed.
///
/// Only the CPU or a PTE can access a page table node using a raw handle. To access the page
/// table node from the kernel code, use the handle [`PageTableNode`].
#[derive(Debug)]
pub(super) struct RawPageTableNode<E: PageTableEntryTrait, C: PagingConstsTrait> {
    raw: Paddr,
    level: PagingLevel,
    _phantom: PhantomData<(E, C)>,
}

impl<E: PageTableEntryTrait, C: PagingConstsTrait> RawPageTableNode<E, C> {
    pub(super) fn paddr(&self) -> Paddr {
        self.raw
    }

    pub(super) fn level(&self) -> PagingLevel {
        self.level
    }

    /// Converts a raw handle to an accessible handle by pertaining the lock.
    pub(super) fn lock(self) -> PageTableNode<E, C> {
        let level = self.level;
        let page: Frame<PageTablePageMeta<E, C>> = self.into();

        // Acquire the lock.
        let meta = page.meta();
        while meta
            .lock
            .compare_exchange(0, 1, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            core::hint::spin_loop();
        }

        debug_assert_eq!(page.meta().level, level);

        PageTableNode::<E, C> { page }
    }

    /// Creates a copy of the handle.
    pub(super) fn clone_shallow(&self) -> Self {
        self.inc_ref_count();

        Self {
            raw: self.raw,
            level: self.level,
            _phantom: PhantomData,
        }
    }

    /// Activates the (root) page table assuming it is the first activation.
    ///
    /// It will not try dropping the last activate page table. It is the same
    /// with [`Self::activate()`] in other senses.
    pub(super) unsafe fn first_activate(&self) {
        use crate::{arch::mm::activate_page_table, mm::CachePolicy};

        self.inc_ref_count();

        activate_page_table(self.raw, CachePolicy::Writeback);
    }

    fn inc_ref_count(&self) {
        // SAFETY: We have a reference count to the page and can safely increase the reference
        // count by one more.
        unsafe {
            inc_frame_ref_count(self.paddr());
        }
    }

    /// Restores the handle from the physical address and level.
    ///
    /// # Safety
    ///
    /// The caller must ensure that the physical address is valid and points to
    /// a forgotten page table node. A forgotten page table node can only be
    /// restored once. The level must match the level of the page table node.
    pub(super) unsafe fn from_raw_parts(paddr: Paddr, level: PagingLevel) -> Self {
        Self {
            raw: paddr,
            level,
            _phantom: PhantomData,
        }
    }
}

impl<E: PageTableEntryTrait, C: PagingConstsTrait> From<RawPageTableNode<E, C>>
    for Frame<PageTablePageMeta<E, C>>
{
    fn from(raw: RawPageTableNode<E, C>) -> Self {
        let raw = ManuallyDrop::new(raw);
        // SAFETY: The physical address in the raw handle is valid and we are
        // transferring the ownership to a new handle. No increment of the reference
        // count is needed.
        unsafe { Frame::<PageTablePageMeta<E, C>>::from_raw(raw.paddr()) }
    }
}

impl<E: PageTableEntryTrait, C: PagingConstsTrait> Drop for RawPageTableNode<E, C> {
    fn drop(&mut self) {
        // SAFETY: The physical address in the raw handle is valid. The restored
        // handle is dropped to decrement the reference count.
        drop(unsafe { Frame::<PageTablePageMeta<E, C>>::from_raw(self.paddr()) });
    }
}

/// A mutable handle to a page table node.
///
/// The page table node can own a set of handles to children, ensuring that the children
/// don't outlive the page table node. Cloning a page table node will create a deep copy
/// of the page table. Dropping the page table node will also drop all handles if the page
/// table node has no references. You can set the page table node as a child of another
/// page table node.
#[derive(Debug)]
pub(super) struct PageTableNode<
    E: PageTableEntryTrait = PageTableEntry,
    C: PagingConstsTrait = PagingConsts,
> {
    page: Frame<PageTablePageMeta<E, C>>,
}

impl<E: PageTableEntryTrait, C: PagingConstsTrait> PageTableNode<E, C> {
    /// Borrows an entry in the node at a given index.
    ///
    /// # Panics
    ///
    /// Panics if the index is not within the bound of
    /// [`nr_subpage_per_huge<C>`].
    pub(super) fn entry(&mut self, idx: usize) -> Entry<'_, E, C> {
        assert!(idx < nr_subpage_per_huge::<C>());
        // SAFETY: The index is within the bound.
        unsafe { Entry::new_at(self, idx) }
    }

    /// Gets the level of the page table node.
    pub(super) fn level(&self) -> PagingLevel {
        self.page.meta().level
    }

    /// Gets the tracking status of the page table node.
    pub(super) fn is_tracked(&self) -> MapTrackingStatus {
        self.page.meta().is_tracked
    }

    /// Allocates a new empty page table node.
    ///
    /// This function returns an owning handle. The newly created handle does not
    /// set the lock bit for performance as it is exclusive and unlocking is an
    /// extra unnecessary expensive operation.
    pub(super) fn alloc(level: PagingLevel, is_tracked: MapTrackingStatus) -> Self {
        let meta = PageTablePageMeta::new_locked(level, is_tracked);
        let page = FrameAllocOptions::new()
            .zeroed(true)
            .alloc_frame_with(meta)
            .expect("Failed to allocate a page table node");
        // The allocated frame is zeroed. Make sure zero is absent PTE.
        debug_assert!(E::new_absent().as_bytes().iter().all(|&b| b == 0));

        Self { page }
    }

    /// Converts the handle into a raw handle to be stored in a PTE or CPU.
    pub(super) fn into_raw(self) -> RawPageTableNode<E, C> {
        let this = ManuallyDrop::new(self);

        // Release the lock.
        this.page.meta().lock.store(0, Ordering::Release);

        // SAFETY: The provided physical address is valid and the level is
        // correct. The reference count is not changed.
        unsafe { RawPageTableNode::from_raw_parts(this.page.start_paddr(), this.page.meta().level) }
    }

    /// Gets a raw handle while still preserving the original handle.
    pub(super) fn clone_raw(&self) -> RawPageTableNode<E, C> {
        let page = ManuallyDrop::new(self.page.clone());

        // SAFETY: The provided physical address is valid and the level is
        // correct. The reference count is increased by one.
        unsafe { RawPageTableNode::from_raw_parts(page.start_paddr(), page.meta().level) }
    }

    /// Gets the number of valid PTEs in the node.
    pub(super) fn nr_children(&self) -> u16 {
        // SAFETY: The lock is held so we have an exclusive access.
        unsafe { *self.page.meta().nr_children.get() }
    }

    /// Reads a non-owning PTE at the given index.
    ///
    /// A non-owning PTE means that it does not account for a reference count
    /// of the a page if the PTE points to a page. The original PTE still owns
    /// the child page.
    ///
    /// # Safety
    ///
    /// The caller must ensure that the index is within the bound.
    unsafe fn read_pte(&self, idx: usize) -> E {
        debug_assert!(idx < nr_subpage_per_huge::<C>());
        let ptr = paddr_to_vaddr(self.page.start_paddr()) as *mut E;
        // SAFETY:
        // - The page table node is alive. The index is inside the bound, so the page table entry is valid.
        // - All page table entries are aligned and accessed with atomic operations only.
        unsafe { load_pte(ptr.add(idx), Ordering::Relaxed) }
    }

    /// Writes a page table entry at a given index.
    ///
    /// This operation will leak the old child if the old PTE is present.
    ///
    /// The child represented by the given PTE will handover the ownership to
    /// the node. The PTE will be rendered invalid after this operation.
    ///
    /// # Safety
    ///
    /// The caller must ensure that:
    ///  1. The index must be within the bound;
    ///  2. The PTE must represent a child compatible with this page table node
    ///     (see [`Child::is_compatible`]).
    unsafe fn write_pte(&mut self, idx: usize, pte: E) {
        debug_assert!(idx < nr_subpage_per_huge::<C>());
        let ptr = paddr_to_vaddr(self.page.start_paddr()) as *mut E;
        // SAFETY:
        // - The page table node is alive. The index is inside the bound, so the page table entry is valid.
        // - All page table entries are aligned and accessed with atomic operations only.
        unsafe { store_pte(ptr.add(idx), pte, Ordering::Release) }
    }

    /// Gets the mutable reference to the number of valid PTEs in the node.
    fn nr_children_mut(&mut self) -> &mut u16 {
        // SAFETY: The lock is held so we have an exclusive access.
        unsafe { &mut *self.page.meta().nr_children.get() }
    }
}

impl<E: PageTableEntryTrait, C: PagingConstsTrait> Drop for PageTableNode<E, C> {
    fn drop(&mut self) {
        // Release the lock.
        self.page.meta().lock.store(0, Ordering::Release);
    }
}

/// The metadata of any kinds of page table pages.
/// Make sure the the generic parameters don't effect the memory layout.
#[derive(Debug)]
pub(in crate::mm) struct PageTablePageMeta<
    E: PageTableEntryTrait = PageTableEntry,
    C: PagingConstsTrait = PagingConsts,
> {
    /// The number of valid PTEs. It is mutable if the lock is held.
    pub nr_children: SyncUnsafeCell<u16>,
    /// The level of the page table page. A page table page cannot be
    /// referenced by page tables of different levels.
    pub level: PagingLevel,
    /// The lock for the page table page.
    pub lock: AtomicU8,
    /// Whether the pages mapped by the node is tracked.
    pub is_tracked: MapTrackingStatus,
    _phantom: core::marker::PhantomData<(E, C)>,
}

/// Describe if the physical address recorded in this page table refers to a
/// page tracked by metadata.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(in crate::mm) enum MapTrackingStatus {
    /// The page table node cannot contain references to any pages. It can only
    /// contain references to child page table nodes.
    NotApplicable,
    /// The mapped pages are not tracked by metadata. If any child page table
    /// nodes exist, they should also be tracked.
    Untracked,
    /// The mapped pages are tracked by metadata. If any child page table nodes
    /// exist, they should also be tracked.
    Tracked,
}

impl<E: PageTableEntryTrait, C: PagingConstsTrait> PageTablePageMeta<E, C> {
    pub fn new_locked(level: PagingLevel, is_tracked: MapTrackingStatus) -> Self {
        Self {
            nr_children: SyncUnsafeCell::new(0),
            level,
            lock: AtomicU8::new(1),
            is_tracked,
            _phantom: PhantomData,
        }
    }
}

// SAFETY: The layout of the `PageTablePageMeta` is ensured to be the same for
// all possible generic parameters. And the layout fits the requirements.
unsafe impl<E: PageTableEntryTrait, C: PagingConstsTrait> AnyFrameMeta for PageTablePageMeta<E, C> {
    fn on_drop(&mut self, reader: &mut VmReader<Infallible>) {
        let nr_children = self.nr_children.get_mut();

        if *nr_children == 0 {
            return;
        }

        let level = self.level;
        let is_tracked = self.is_tracked;

        // Drop the children.
        while let Ok(pte) = reader.read_once::<E>() {
            // Here if we use directly `Child::from_pte` we would experience a
            // 50% increase in the overhead of the `drop` function. It seems that
            // Rust is very conservative about inlining and optimizing dead code
            // for `unsafe` code. So we manually inline the function here.
            if pte.is_present() {
                let paddr = pte.paddr();
                if !pte.is_last(level) {
                    // SAFETY: The PTE points to a page table node. The ownership
                    // of the child is transferred to the child then dropped.
                    drop(unsafe { Frame::<Self>::from_raw(paddr) });
                } else if is_tracked == MapTrackingStatus::Tracked {
                    // SAFETY: The PTE points to a tracked page. The ownership
                    // of the child is transferred to the child then dropped.
                    drop(unsafe { Frame::<dyn AnyFrameMeta>::from_raw(paddr) });
                }
            }
        }
    }
}
