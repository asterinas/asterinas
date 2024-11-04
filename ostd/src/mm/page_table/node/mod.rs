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

use core::{marker::PhantomData, mem::ManuallyDrop, sync::atomic::Ordering};

pub(in crate::mm) use self::{child::Child, entry::Entry};
use super::{nr_subpage_per_huge, PageTableEntryTrait};
use crate::{
    arch::mm::{PageTableEntry, PagingConsts},
    mm::{
        paddr_to_vaddr,
        page::{
            self, inc_page_ref_count,
            meta::{MapTrackingStatus, PageMeta, PageTablePageMeta, PageUsage},
            DynPage, Page,
        },
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
    level: PagingLevel,
    _phantom: PhantomData<(E, C)>,
}

impl<E: PageTableEntryTrait, C: PagingConstsTrait> RawPageTableNode<E, C>
where
    [(); C::NR_LEVELS as usize]:,
{
    pub(super) fn paddr(&self) -> Paddr {
        self.raw
    }

    pub(super) fn level(&self) -> PagingLevel {
        self.level
    }

    /// Converts a raw handle to an accessible handle by pertaining the lock.
    pub(super) fn lock(self) -> PageTableNode<E, C> {
        let level = self.level;
        let page: Page<PageTablePageMeta<E, C>> = self.into();

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
    ///
    /// # Panics
    ///
    /// Only top-level page tables can be activated using this function.
    pub(crate) unsafe fn activate(&self) {
        use crate::{
            arch::mm::{activate_page_table, current_page_table_paddr},
            mm::CachePolicy,
        };

        assert_eq!(self.level, C::NR_LEVELS);

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
            level: C::NR_LEVELS,
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
            inc_page_ref_count(self.paddr());
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

        // Release the lock.
        this.page.meta().lock.store(0, Ordering::Release);

        // SAFETY: The provided physical address is valid and the level is
        // correct. The reference count is not changed.
        unsafe { RawPageTableNode::from_raw_parts(this.page.paddr(), this.page.meta().level) }
    }

    /// Gets a raw handle while still preserving the original handle.
    pub(super) fn clone_raw(&self) -> RawPageTableNode<E, C> {
        let page = ManuallyDrop::new(self.page.clone());

        // SAFETY: The provided physical address is valid and the level is
        // correct. The reference count is increased by one.
        unsafe { RawPageTableNode::from_raw_parts(page.paddr(), page.meta().level) }
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
        let ptr = paddr_to_vaddr(self.page.paddr()) as *const E;
        // SAFETY: The index is within the bound and the PTE is plain-old-data.
        unsafe { ptr.add(idx).read() }
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
        let ptr = paddr_to_vaddr(self.page.paddr()) as *mut E;
        // SAFETY: The index is within the bound and the PTE is plain-old-data.
        unsafe { ptr.add(idx).write(pte) }
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

impl<E: PageTableEntryTrait, C: PagingConstsTrait> PageMeta for PageTablePageMeta<E, C>
where
    [(); C::NR_LEVELS as usize]:,
{
    const USAGE: PageUsage = PageUsage::PageTable;

    fn on_drop(page: &mut Page<Self>) {
        // SAFETY: This is the last reference so we have an exclusive access.
        let nr_children = unsafe { *page.meta().nr_children.get() };

        if nr_children == 0 {
            return;
        }

        let paddr = page.paddr();
        let level = page.meta().level;
        let is_tracked = page.meta().is_tracked;

        // Drop the children.
        for i in 0..nr_subpage_per_huge::<C>() {
            // SAFETY: The index is within the bound and PTE is plain-old-data. The
            // address is aligned as well. We also have an exclusive access ensured
            // by reference counting.
            let pte_ptr = unsafe { (paddr_to_vaddr(paddr) as *const E).add(i) };
            // SAFETY: The pointer is valid and the PTE is plain-old-data.
            let pte = unsafe { pte_ptr.read() };

            // Here if we use directly `Child::from_pte` we would experience a
            // 50% increase in the overhead of the `drop` function. It seems that
            // Rust is very conservative about inlining and optimizing dead code
            // for `unsafe` code. So we manually inline the function here.
            if pte.is_present() {
                let paddr = pte.paddr();
                if !pte.is_last(level) {
                    // SAFETY: The PTE points to a page table node. The ownership
                    // of the child is transferred to the child then dropped.
                    drop(unsafe { Page::<Self>::from_raw(paddr) });
                } else if is_tracked == MapTrackingStatus::Tracked {
                    // SAFETY: The PTE points to a tracked page. The ownership
                    // of the child is transferred to the child then dropped.
                    drop(unsafe { DynPage::from_raw(paddr) });
                }
            }
        }
    }
}
