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
        paddr_to_vaddr,
        page::{self, meta::PageMeta, Page},
        Paddr, PagingConstsTrait, PagingLevel, PAGE_SIZE,
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
pub(super) struct RawPageTableNode<E: PageTableEntryTrait, C: PagingConstsTrait>
where
    [(); C::NR_LEVELS as usize]:,
{
    raw: Paddr,
    _phantom: PhantomData<(E, C)>,
}

impl<E: PageTableEntryTrait, C: PagingConstsTrait> RawPageTableNode<E, C>
where
    [(); C::NR_LEVELS as usize]:,
{
    pub(super) fn paddr(&self) -> Paddr {
        self.raw
    }

    /// Converts a raw handle to an accessible handle by pertaining the lock.
    pub(super) fn lock(self) -> PageTableNode<E, C> {
        let page: Page<PageTablePageMeta<E, C>> = self.into();

        // Acquire the lock.
        while page
            .meta()
            .lock
            .compare_exchange(0, 1, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            core::hint::spin_loop();
        }

        PageTableNode::<E, C> { page }
    }

    /// Creates a copy of the handle.
    pub(super) fn clone_shallow(&self) -> Self {
        self.inc_ref_count();

        Self {
            raw: self.raw,
            _phantom: PhantomData,
        }
    }

    /// Activates the page table assuming it is a root page table.
    ///
    /// Here we ensure not dropping an active page table by making a
    /// processor a page table owner. When activating a page table, the
    /// reference count of the last activated page table is decremented.
    /// And that of the current page table is incremented.
    ///
    /// # Safety
    ///
    /// The caller must ensure that the page table to be activated has
    /// proper mappings for the kernel and has the correct const parameters
    /// matching the current CPU.
    pub(crate) unsafe fn activate(&self) {
        use crate::{
            arch::mm::{activate_page_table, current_page_table_paddr},
            mm::CachePolicy,
        };

        let last_activated_paddr = current_page_table_paddr();

        if last_activated_paddr == self.raw {
            return;
        }

        activate_page_table(self.raw, CachePolicy::Writeback);

        // Increment the reference count of the current page table.
        self.inc_ref_count();

        // Restore and drop the last activated page table.
        drop(Self {
            raw: last_activated_paddr,
            _phantom: PhantomData,
        });
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
            Page::<PageTablePageMeta<E, C>>::inc_ref_count(self.paddr());
        }
    }

    /// Restore the handle to a page table node from a physical address.
    ///
    /// # Safety
    ///
    /// The caller must ensure that the physical address is valid and points to
    /// a forgotten page table node. A forgotten page table node can only be
    /// restored once.
    unsafe fn from_paddr(paddr: Paddr) -> Self {
        Self {
            raw: paddr,
            _phantom: PhantomData,
        }
    }
}

impl<E: PageTableEntryTrait, C: PagingConstsTrait> From<RawPageTableNode<E, C>>
    for Page<PageTablePageMeta<E, C>>
where
    [(); C::NR_LEVELS as usize]:,
{
    fn from(raw: RawPageTableNode<E, C>) -> Self {
        let raw = ManuallyDrop::new(raw);
        // SAFETY: The physical address in the raw handle is valid and we are
        // transferring the ownership to a new handle. No increment of the reference
        // count is needed.
        unsafe { Page::<PageTablePageMeta<E, C>>::from_raw(raw.paddr()) }
    }
}

impl<E: PageTableEntryTrait, C: PagingConstsTrait> Drop for RawPageTableNode<E, C>
where
    [(); C::NR_LEVELS as usize]:,
{
    fn drop(&mut self) {
        // SAFETY: The physical address in the raw handle is valid. The restored
        // handle is dropped to decrement the reference count.
        drop(unsafe { Page::<PageTablePageMeta<E, C>>::from_raw(self.paddr()) });
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
> where
    [(); C::NR_LEVELS as usize]:,
{
    page: Page<PageTablePageMeta<E, C>>,
}

impl<E: PageTableEntryTrait, C: PagingConstsTrait> PageTableNode<E, C>
where
    [(); C::NR_LEVELS as usize]:,
{
    /// Borrows an entry in the node at a given index.
    ///
    /// The index must be within the bound of [`nr_subpage_per_huge<C>`].
    pub(super) fn entry(&mut self, idx: usize) -> Entry<'_, E, C> {
        Entry::new_at(self, idx)
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
        let page = page::allocator::alloc_single::<PageTablePageMeta<E, C>>(meta).unwrap();

        // Zero out the page table node.
        let ptr = paddr_to_vaddr(page.paddr()) as *mut u8;
        // SAFETY: The page is exclusively owned here. Pointers are valid also.
        // We rely on the fact that 0 represents an absent entry to speed up `memset`.
        unsafe { core::ptr::write_bytes(ptr, 0, PAGE_SIZE) };
        debug_assert!(E::new_absent().as_bytes().iter().all(|&b| b == 0));

        Self { page }
    }

    /// Converts the handle into a raw handle to be stored in a PTE or CPU.
    pub(super) fn into_raw(self) -> RawPageTableNode<E, C> {
        let this = ManuallyDrop::new(self);

        let raw = this.page.paddr();

        this.page.meta().lock.store(0, Ordering::Release);

        RawPageTableNode {
            raw,
            _phantom: PhantomData,
        }
    }

    /// Gets a raw handle while still preserving the original handle.
    pub(super) fn clone_raw(&self) -> RawPageTableNode<E, C> {
        core::mem::forget(self.page.clone());

        RawPageTableNode {
            raw: self.page.paddr(),
            _phantom: PhantomData,
        }
    }

    /// Gets the number of valid PTEs in the node.
    pub(super) fn nr_children(&self) -> u16 {
        // SAFETY: The lock is held so we have an exclusive access.
        unsafe { *self.page.meta().nr_children.get() }
    }

    /// Read a non-owning PTE at the entry.
    ///
    /// A non-owning PTE means that it does not account for a reference count
    /// of the a page if the PTE points to a page.
    fn read_pte(&self, idx: usize) -> E {
        // It should be ensured by the cursor.
        debug_assert!(idx < nr_subpage_per_huge::<C>());

        let ptr = paddr_to_vaddr(self.page.paddr()) as *const E;

        // SAFETY: the index is within the bound and PTE is plain-old-data.
        unsafe { ptr.add(idx).read() }
    }

    /// Writes a page table entry at a given index.
    ///
    /// This operation will leak the old child if the old PTE is present.
    fn write_pte(&mut self, idx: usize, pte: E) {
        // It should be ensured by the cursor.
        debug_assert!(idx < nr_subpage_per_huge::<C>());

        let ptr = paddr_to_vaddr(self.page.paddr()) as *mut E;

        // SAFETY: the index is within the bound and PTE is plain-old-data.
        unsafe { ptr.add(idx).write(pte) };
    }

    /// Gets the mutable reference to the number of valid PTEs in the node.
    fn nr_children_mut(&mut self) -> &mut u16 {
        // SAFETY: The lock is held so we have an exclusive access.
        unsafe { &mut *self.page.meta().nr_children.get() }
    }
}

impl<E: PageTableEntryTrait, C: PagingConstsTrait> Drop for PageTableNode<E, C>
where
    [(); C::NR_LEVELS as usize]:,
{
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
> where
    [(); C::NR_LEVELS as usize]:,
{
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

impl<E: PageTableEntryTrait, C: PagingConstsTrait> PageTablePageMeta<E, C>
where
    [(); C::NR_LEVELS as usize]:,
{
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

impl<E: PageTableEntryTrait, C: PagingConstsTrait> PageMeta for PageTablePageMeta<E, C>
where
    [(); C::NR_LEVELS as usize]:,
{
    fn on_drop(&mut self, paddr: Paddr) {
        // SAFETY: The lock is held so we have an exclusive access.
        let nr_children = unsafe { *self.nr_children.get() };

        if nr_children == 0 {
            return;
        }

        let level = self.level;
        let is_tracked = self.is_tracked;

        // Drop the children.
        for i in 0..nr_subpage_per_huge::<C>() {
            // SAFETY: The index is within the bound and PTE is plain-old-data. The
            // address is aligned as well. We also have an exclusive access ensured
            // by reference counting.
            let pte_ptr = unsafe { (paddr_to_vaddr(paddr) as *const E).add(i) };
            // SAFETY: The pointer is valid and the PTE is plain-old-data.
            let pte = unsafe { pte_ptr.read() };
            // SAFETY: The PTE is valid and the level and tracking status are correct.
            // The ownership of the child is transferred to the child then dropped.
            let _child = unsafe { Child::from_pte(pte, level, is_tracked) };
        }
    }
}
