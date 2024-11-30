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
    ops::Deref,
    sync::atomic::{AtomicU8, Ordering},
};

pub(in crate::mm) use self::{child::Child, entry::Entry};
use super::{nr_subpage_per_huge, PageTableEntryTrait};
use crate::mm::{
    frame::{meta::AnyFrameMeta, Frame, FrameRef},
    paddr_to_vaddr,
    page_table::{load_pte, store_pte},
    FrameAllocOptions, Infallible, Paddr, PagingConstsTrait, PagingLevel, VmReader,
};

/// A smart pointer to a page table node.
///
/// This smart pointer is an owner of a page table node. Thus creating and
/// dropping it will affect the reference count of the page table node. If
/// dropped it as the last reference, the page table node and subsequent
/// children will be freed.
///
/// [`PageTableNode`] is read-only. To modify the page table node, lock and use
/// [`PageTableGuard`].
pub(super) type PageTableNode<E, C> = Frame<PageTablePageMeta<E, C>>;

/// A reference to a page table node.
pub(super) type PageTableNodeRef<'a, E, C> = FrameRef<'a, PageTablePageMeta<E, C>>;

impl<E: PageTableEntryTrait, C: PagingConstsTrait> PageTableNode<E, C> {
    pub(super) fn level(&self) -> PagingLevel {
        self.meta().level
    }

    pub(super) fn is_tracked(&self) -> MapTrackingStatus {
        self.meta().is_tracked
    }

    /// Allocates a new empty page table node.
    ///
    /// This function returns a locked owning guard.
    pub(super) fn alloc(level: PagingLevel, is_tracked: MapTrackingStatus) -> Self {
        let meta = PageTablePageMeta::new(level, is_tracked);
        let frame = FrameAllocOptions::new()
            .zeroed(true)
            .alloc_frame_with(meta)
            .expect("Failed to allocate a page table node");
        // The allocated frame is zeroed. Make sure zero is absent PTE.
        debug_assert!(E::new_absent().as_bytes().iter().all(|&b| b == 0));

        frame
    }

    /// Locks the page table node.
    pub(super) fn lock(&self) -> PageTableGuard<'_, E, C> {
        while self
            .meta()
            .lock
            .compare_exchange(0, 1, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            core::hint::spin_loop();
        }

        PageTableGuard::<'_, E, C> {
            inner: self.borrow(),
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

        assert_eq!(self.level(), C::NR_LEVELS);

        let last_activated_paddr = current_page_table_paddr();
        if last_activated_paddr == self.start_paddr() {
            return;
        }

        activate_page_table(self.clone().into_raw(), CachePolicy::Writeback);

        // Restore and drop the last activated page table.
        // SAFETY: The physical address is valid and points to a forgotten page table node.
        drop(unsafe { Self::from_raw(last_activated_paddr) });
    }

    /// Activates the (root) page table assuming it is the first activation.
    ///
    /// It will not try dropping the last activate page table. It is the same
    /// with [`Self::activate()`] in other senses.
    pub(super) unsafe fn first_activate(&self) {
        use crate::{arch::mm::activate_page_table, mm::CachePolicy};

        activate_page_table(self.clone().into_raw(), CachePolicy::Writeback);
    }
}

/// A guard that holds the lock of a page table node.
#[derive(Debug)]
pub(super) struct PageTableGuard<'a, E: PageTableEntryTrait, C: PagingConstsTrait> {
    inner: PageTableNodeRef<'a, E, C>,
}

impl<'a, E: PageTableEntryTrait, C: PagingConstsTrait> PageTableGuard<'a, E, C> {
    /// Borrows an entry in the node at a given index.
    ///
    /// # Panics
    ///
    /// Panics if the index is not within the bound of
    /// [`nr_subpage_per_huge<C>`].
    pub(super) fn entry<'s>(&'s mut self, idx: usize) -> Entry<'s, 'a, E, C> {
        assert!(idx < nr_subpage_per_huge::<C>());
        // SAFETY: The index is within the bound.
        unsafe { Entry::new_at(self, idx) }
    }

    /// Converts the guard into a raw physical address.
    ///
    /// It will not release the lock. It may be paired with [`Self::from_raw_paddr`]
    /// to manually manage pointers.
    pub(super) fn into_raw_paddr(self) -> Paddr {
        self.start_paddr()
    }

    /// Converts a raw physical address to a guard.
    ///
    /// # Safety
    ///
    /// The caller must ensure that the physical address is valid and points to
    /// a forgotten page table node that is locked (see [`Self::into_raw_paddr`]).
    pub(super) unsafe fn from_raw_paddr(paddr: Paddr) -> Self {
        Self {
            // SAFETY: The caller ensures safety.
            inner: unsafe { PageTableNodeRef::borrow_paddr(paddr) },
        }
    }

    /// Gets the number of valid PTEs in the node.
    pub(super) fn nr_children(&self) -> u16 {
        // SAFETY: The lock is held so we have an exclusive access.
        unsafe { *self.meta().nr_children.get() }
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
        let ptr = paddr_to_vaddr(self.start_paddr()) as *mut E;
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
        let ptr = paddr_to_vaddr(self.start_paddr()) as *mut E;
        // SAFETY:
        // - The page table node is alive. The index is inside the bound, so the page table entry is valid.
        // - All page table entries are aligned and accessed with atomic operations only.
        unsafe { store_pte(ptr.add(idx), pte, Ordering::Release) }
    }

    /// Gets the mutable reference to the number of valid PTEs in the node.
    fn nr_children_mut(&mut self) -> &mut u16 {
        // SAFETY: The lock is held so we have an exclusive access.
        unsafe { &mut *self.meta().nr_children.get() }
    }
}

impl<'a, E: PageTableEntryTrait, C: PagingConstsTrait> Deref for PageTableGuard<'a, E, C> {
    type Target = PageTableNodeRef<'a, E, C>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<E: PageTableEntryTrait, C: PagingConstsTrait> Drop for PageTableGuard<'_, E, C> {
    fn drop(&mut self) {
        self.inner.meta().lock.store(0, Ordering::Release);
    }
}

/// The metadata of any kinds of page table pages.
/// Make sure the the generic parameters don't effect the memory layout.
#[derive(Debug)]
pub(in crate::mm) struct PageTablePageMeta<E: PageTableEntryTrait, C: PagingConstsTrait> {
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
    pub fn new(level: PagingLevel, is_tracked: MapTrackingStatus) -> Self {
        Self {
            nr_children: SyncUnsafeCell::new(0),
            level,
            lock: AtomicU8::new(0),
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
