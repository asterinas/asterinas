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
mod mcs;

use alloc::boxed::Box;
use core::{cell::SyncUnsafeCell, marker::PhantomData, ops::Deref, pin::Pin};

pub(in crate::mm) use self::{
    child::{Child, ChildRef},
    entry::Entry,
};
use super::{nr_subpage_per_huge, PageTableConfig, PageTableEntryTrait};
use crate::{
    mm::{
        frame::{meta::AnyFrameMeta, Frame, FrameRef},
        paddr_to_vaddr,
        page_table::zeroed_pt_pool,
        vm_space::Status,
        FrameAllocOptions, Infallible, PageProperty, PagingConstsTrait, PagingLevel, VmReader,
    },
    task::atomic_mode::InAtomicMode,
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
pub(super) type PageTableNode<C> = Frame<PageTablePageMeta<C>>;

impl<C: PageTableConfig> PageTableNode<C> {
    pub(super) fn level(&self) -> PagingLevel {
        self.meta().level
    }

    /// Allocates a new empty page table node.
    pub(super) fn alloc(level: PagingLevel) -> Self {
        // The allocated frame is zeroed. Make sure zero is absent PTE.
        debug_assert_eq!(C::E::new_absent().as_usize(), 0);

        zeroed_pt_pool::alloc(level)
    }

    /// Allocates a new page table node filled with the given status.
    pub(super) fn alloc_marked(level: PagingLevel, status: Status) -> Self {
        let mut meta = PageTablePageMeta::new(level);
        *meta.nr_children.get_mut() = nr_subpage_per_huge::<C>() as u16;
        let frame = FrameAllocOptions::new()
            .zeroed(false)
            .alloc_frame_with(meta)
            .expect("Failed to allocate a page table node");
        let ptr = paddr_to_vaddr(frame.start_paddr()) as *mut C::E;

        let paddr = status.into_raw_inner();
        let status = C::E::new_page(paddr, level, PageProperty::new_absent());

        for i in 0..nr_subpage_per_huge::<C>() {
            // SAFETY: The page table node is not typed. And the index is within the bound.
            unsafe { ptr.add(i).write(status) };
        }

        frame
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

        // SAFETY: The safety is upheld by the caller.
        unsafe { activate_page_table(self.clone().into_raw(), CachePolicy::Writeback) };

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

        // SAFETY: The safety is upheld by the caller.
        unsafe { activate_page_table(self.clone().into_raw(), CachePolicy::Writeback) };
    }
}

/// A reference to a page table node.
pub(super) type PageTableNodeRef<'a, C> = FrameRef<'a, PageTablePageMeta<C>>;

impl<'a, C: PageTableConfig> PageTableNodeRef<'a, C> {
    /// Locks the page table node.
    ///
    /// An atomic mode guard is required to
    ///  1. prevent deadlocks;
    ///  2. provide a lifetime (`'rcu`) that the nodes are guaranteed to outlive.
    pub(super) fn lock<'rcu>(self, guard: &'rcu dyn InAtomicMode) -> PageTableGuard<'rcu, C>
    where
        'a: 'rcu,
    {
        let _ = guard;

        let node = Box::pin(mcs::Node::new());

        // SAFETY: The node is new.
        unsafe { node.as_ref().lock(&self.meta().lock) };

        // SAFETY: Lock is held. So it is exclusive.
        unsafe {
            self.meta().node.get().write(Some(node));
        }

        PageTableGuard::<'rcu, C> { inner: self }
    }

    /// Creates a new [`PageTableGuard`] without checking if the page table lock is held.
    ///
    /// # Safety
    ///
    /// This function must be called if this task logically holds the lock.
    ///
    /// Calling this function when a guard is already created is undefined behavior
    /// unless that guard was already forgotten.
    pub(super) unsafe fn make_guard_unchecked<'rcu>(
        self,
        _guard: &'rcu dyn InAtomicMode,
    ) -> PageTableGuard<'rcu, C>
    where
        'a: 'rcu,
    {
        PageTableGuard { inner: self }
    }
}

/// A guard that holds the lock of a page table node.
#[derive(Debug)]
pub(super) struct PageTableGuard<'rcu, C: PageTableConfig> {
    inner: PageTableNodeRef<'rcu, C>,
}

impl<'rcu, C: PageTableConfig> PageTableGuard<'rcu, C> {
    /// Borrows an entry in the node at a given index.
    ///
    /// # Panics
    ///
    /// Panics if the index is not within the bound of
    /// [`nr_subpage_per_huge<C>`].
    pub(super) fn entry(&mut self, idx: usize) -> Entry<'_, 'rcu, C> {
        assert!(idx < nr_subpage_per_huge::<C>());
        // SAFETY: The index is within the bound.
        unsafe { Entry::new_at(self, idx) }
    }

    /// Gets the number of valid PTEs in the node.
    pub(super) fn nr_children(&self) -> u16 {
        // SAFETY: The lock is held so we have an exclusive access.
        unsafe { *self.meta().nr_children.get() }
    }

    /// Returns if the page table node is detached from its parent.
    pub(super) fn stray_mut(&mut self) -> &mut bool {
        // SAFETY: The lock is held so we have an exclusive access.
        unsafe { &mut *self.meta().stray.get() }
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
    pub(super) unsafe fn read_pte(&self, idx: usize) -> C::E {
        debug_assert!(idx < nr_subpage_per_huge::<C>());
        let ptr = paddr_to_vaddr(self.start_paddr()) as *mut C::E;
        // SAFETY:
        // - The page table node is alive. The index is inside the bound, so the page table entry is valid.
        // - All page table entries are aligned and accessed with atomic operations only.
        unsafe { ptr.add(idx).read_volatile() }
    }

    /// Writes a page table entry at a given index.
    ///
    /// This operation will leak the old child if the old PTE is present.
    ///
    /// # Safety
    ///
    /// The caller must ensure that:
    ///  1. The index must be within the bound;
    ///  2. The PTE must represent a [`Child`] in the same [`PageTableConfig`]
    ///     and at the right paging level (`self.level() - 1`).
    ///  3. The page table node will have the ownership of the [`Child`]
    ///     after this method.
    pub(super) unsafe fn write_pte(&mut self, idx: usize, pte: C::E) {
        debug_assert!(idx < nr_subpage_per_huge::<C>());
        let ptr = paddr_to_vaddr(self.start_paddr()) as *mut C::E;
        // SAFETY:
        // - The page table node is alive. The index is inside the bound, so the page table entry is valid.
        // - All page table entries are aligned and accessed with atomic operations only.
        unsafe { ptr.add(idx).write_volatile(pte) };
    }

    /// Gets the mutable reference to the number of valid PTEs in the node.
    fn nr_children_mut(&mut self) -> &mut u16 {
        // SAFETY: The lock is held so we have an exclusive access.
        unsafe { &mut *self.meta().nr_children.get() }
    }
}

impl<'rcu, C: PageTableConfig> Deref for PageTableGuard<'rcu, C> {
    type Target = PageTableNodeRef<'rcu, C>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<C: PageTableConfig> Drop for PageTableGuard<'_, C> {
    fn drop(&mut self) {
        // SAFETY: Lock is held. So it is exclusive.
        let node = unsafe { self.meta().node.get().replace(None) }.unwrap();

        // Release the lock.
        // SAFETY:
        //  - The lock stays at the metadata slot so it's pinned.
        //  - The acquire method ensures that the node matches the lock.
        unsafe { node.as_ref().unlock(&self.meta().lock) };
    }
}

/// The metadata of any kinds of page table pages.
/// Make sure the the generic parameters don't effect the memory layout.
#[derive(Debug)]
pub(in crate::mm) struct PageTablePageMeta<C: PageTableConfig> {
    /// The number of valid PTEs. It is mutable if the lock is held.
    pub nr_children: SyncUnsafeCell<u16>,
    /// If the page table is detached from its parent.
    ///
    /// A page table can be detached from its parent while still being accessed,
    /// since we use a RCU scheme to recycle page tables. If this flag is set,
    /// it means that the parent is recycling the page table.
    pub stray: SyncUnsafeCell<bool>,
    /// The level of the page table page. A page table page cannot be
    /// referenced by page tables of different levels.
    pub level: PagingLevel,
    /// The lock for the page table page.
    lock: mcs::LockBody,
    node: SyncUnsafeCell<Option<Pin<Box<mcs::Node>>>>,
    _phantom: core::marker::PhantomData<C>,
}

impl<C: PageTableConfig> PageTablePageMeta<C> {
    pub fn new(level: PagingLevel) -> Self {
        Self {
            nr_children: SyncUnsafeCell::new(0),
            stray: SyncUnsafeCell::new(false),
            level,
            lock: mcs::LockBody::new(),
            node: SyncUnsafeCell::new(None),
            _phantom: PhantomData,
        }
    }
}

// FIXME: The safe APIs in the `page_table/node` module allow `Child::Frame`s with
// arbitrary addresses to be stored in the page table nodes. Therefore, they may not
// be valid `C::Item`s. The soundness of the following `on_drop` implementation must
// be reasoned in conjunction with the `page_table/cursor` implementation.
unsafe impl<C: PageTableConfig> AnyFrameMeta for PageTablePageMeta<C> {
    fn on_drop(&mut self, reader: &mut VmReader<Infallible>) {
        let nr_children = self.nr_children.get_mut();
        if *nr_children == 0 {
            return;
        }

        let level = self.level;
        let range = if level == C::NR_LEVELS {
            C::TOP_LEVEL_INDEX_RANGE.clone()
        } else {
            0..nr_subpage_per_huge::<C>()
        };

        // Drop the children.
        reader.skip(range.start * size_of::<C::E>());
        for _ in range {
            // Non-atomic read is OK because we have mutable access.
            let pte = reader.read_once::<C::E>().unwrap();
            if pte.is_present() {
                let paddr = pte.paddr();
                // As a fast path, we can ensure that the type of the child frame
                // is `Self` if the PTE points to a child page table. Then we don't
                // need to check the vtable for the drop method.
                if !pte.is_last(level) {
                    // SAFETY: The PTE points to a page table node. The ownership
                    // of the child is transferred to the child then dropped.
                    drop(unsafe { Frame::<Self>::from_raw(paddr) });
                } else {
                    // SAFETY: The PTE points to a mapped item. The ownership
                    // of the item is transferred here then dropped.
                    drop(unsafe { C::item_from_raw(paddr, level, pte.prop()) });
                }
            }
        }
    }
}
