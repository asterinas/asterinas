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
    any::TypeId,
    cell::SyncUnsafeCell,
    marker::PhantomData,
    ops::Deref,
    sync::atomic::{AtomicU8, Ordering},
};

use ostd_pod::Pod;

pub(in crate::mm) use self::{
    child::{Child, ChildRef},
    entry::Entry,
};
use super::{nr_subpage_per_huge, PageTableConfig, PageTableEntryTrait};
use crate::mm::{
    frame::{meta::AnyFrameMeta, Frame, FrameRef},
    paddr_to_vaddr,
    page_table::{load_pte, store_pte},
    vm_space::UserPtConfig,
    FrameAllocOptions, Infallible, PagingConstsTrait, PagingLevel, VmReader,
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
    ///
    /// This function returns a locked owning guard.
    pub(super) fn alloc(level: PagingLevel) -> Self {
        let meta = PageTablePageMeta::new(level);
        let frame = FrameAllocOptions::new()
            .zeroed(true)
            .alloc_frame_with(meta)
            .expect("Failed to allocate a page table node");
        // The allocated frame is zeroed. Make sure zero is absent PTE.
        debug_assert!(C::E::new_absent().as_bytes().iter().all(|&b| b == 0));

        frame
    }

    /// Locks the page table node.
    pub(super) fn lock(&self) -> PageTableGuard<'_, C> {
        while self
            .meta()
            .lock
            .compare_exchange(0, 1, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            core::hint::spin_loop();
        }

        PageTableGuard::<'_, C> {
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

/// A reference to a page table node.
pub(super) type PageTableNodeRef<'a, C> = FrameRef<'a, PageTablePageMeta<C>>;

impl<'a, C: PageTableConfig> PageTableNodeRef<'a, C> {
    /// Creates a new [`PageTableGuard`] without checking if the page table lock is held.
    ///
    /// # Safety
    ///
    /// This function must be called if this task logically holds the lock.
    ///
    /// Calling this function when a guard is already created is undefined behavior
    /// unless that guard was already forgotten.
    pub(super) unsafe fn make_guard_unchecked(self) -> PageTableGuard<'a, C> {
        PageTableGuard { inner: self }
    }
}

/// A guard that holds the lock of a page table node.
#[derive(Debug)]
pub(super) struct PageTableGuard<'a, C: PageTableConfig> {
    inner: PageTableNodeRef<'a, C>,
}

impl<'a, C: PageTableConfig> PageTableGuard<'a, C> {
    /// Borrows an entry in the node at a given index.
    ///
    /// # Panics
    ///
    /// Panics if the index is not within the bound of
    /// [`nr_subpage_per_huge<C>`].
    pub(super) fn entry<'s>(&'s mut self, idx: usize) -> Entry<'s, 'a, C> {
        assert!(idx < nr_subpage_per_huge::<C>());
        // SAFETY: The index is within the bound.
        unsafe { Entry::new_at(self, idx) }
    }

    /// Gets the number of valid PTEs in the node.
    pub(super) fn nr_children(&self) -> u16 {
        // SAFETY: The lock is held so we have an exclusive access.
        unsafe { *self.meta().nr_children.get() }
    }

    /// If the page table node is detached from its parent.
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
    ///  2. The PTE must represent a valid [`Child`] whose level is compatible
    ///     with the page table node.
    pub(super) unsafe fn write_pte(&mut self, idx: usize, pte: C::E) {
        debug_assert!(idx < nr_subpage_per_huge::<C>());
        let ptr = paddr_to_vaddr(self.start_paddr()) as *mut C::E;
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

impl<'a, C: PageTableConfig> Deref for PageTableGuard<'a, C> {
    type Target = PageTableNodeRef<'a, C>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<C: PageTableConfig> Drop for PageTableGuard<'_, C> {
    fn drop(&mut self) {
        self.inner.meta().lock.store(0, Ordering::Release);
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
    pub lock: AtomicU8,
    _phantom: core::marker::PhantomData<C>,
}

impl<C: PageTableConfig> PageTablePageMeta<C> {
    pub fn new(level: PagingLevel) -> Self {
        Self {
            nr_children: SyncUnsafeCell::new(0),
            stray: SyncUnsafeCell::new(false),
            level,
            lock: AtomicU8::new(0),
            _phantom: PhantomData,
        }
    }
}

// SAFETY: We can read the page table node because the page table pages are
// accessed as untyped memory.
unsafe impl<C: PageTableConfig> AnyFrameMeta for PageTablePageMeta<C> {
    fn on_drop(&mut self, reader: &mut VmReader<Infallible>) {
        let nr_children = self.nr_children.get_mut();

        if *nr_children == 0 {
            return;
        }

        let level = self.level;

        // Drop the children.
        let range = if TypeId::of::<C>() == TypeId::of::<UserPtConfig>() && level == C::NR_LEVELS {
            // Only the user part. The kernel part is not reference-counted.
            0..nr_subpage_per_huge::<C>() / 2
        } else {
            0..nr_subpage_per_huge::<C>()
        };
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
                    drop(unsafe { C::item_from_raw(paddr, level) });
                }
            }
        }
    }
}
