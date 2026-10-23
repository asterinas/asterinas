// SPDX-License-Identifier: MPL-2.0

//! This module defines page table node abstractions and the handle.
//!
//! The page table node is also frequently referred to as a page table in many architectural
//! documentations. It is a contiguous segment of frames containing page table entries (PTEs) that map
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
//! Each backing frame has its own lock, child count, and stray flag. A node reference
//! borrows the entire allocation; a guard owns locks for a contiguous range of its frames.
//! Locks within a node are acquired in increasing frame order. A guard cannot grow its
//! range: doing so after descending into children could invert the lock order.
//!
//! The locks exclude other kernel accesses, but the MMU may still access PTEs concurrently.
//! PTE accesses are atomic, and children are initialized before publishing their parent PTE.
//!

mod entry;
mod iter;
mod pte_state;

#[cfg(ktest)]
mod test;

use core::{
    cell::SyncUnsafeCell,
    marker::PhantomData,
    ops::{Deref, Range},
    sync::atomic::{AtomicU8, Ordering},
};

pub(in crate::mm) use self::{
    entry::Entry,
    pte_state::{PteState, PteStateRef},
};
use super::{PageTableConfig, PteTrait, nr_ptes};
use crate::{
    mm::{
        FrameAllocOptions, HasPaddr, Infallible, PAGE_SIZE, Paddr, PagingConstsTrait, PagingLevel,
        Segment, VmReader,
        frame::{FrameRef, meta::AnyFrameMeta},
        kspace::LINEAR_MAPPING_BASE_VADDR,
        nr_levels, paddr_to_vaddr,
        page_table::{PteScalar, load_pte, store_pte},
        page_table_node_size,
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
#[derive(Clone, Debug)]
pub(crate) struct PageTableNode<C: PageTableConfig> {
    // All frames are owned together, but each frame has its own lock.
    // Each frame's drop hook releases the children owned by its entries.
    frames: Segment<PageTablePageMeta<C>>,
}

impl<C: PageTableConfig> HasPaddr for PageTableNode<C> {
    fn paddr(&self) -> Paddr {
        self.frames.paddr()
    }
}

impl<C: PageTableConfig> PageTableNode<C> {
    pub(super) fn level(&self) -> PagingLevel {
        self.borrow().level()
    }

    pub(super) fn borrow(&self) -> PageTableNodeRef<'_, C> {
        // SAFETY: All frames of this node outlive the returned reference.
        unsafe { PageTableNodeRef::borrow_paddr(self.paddr()) }
    }

    fn into_raw(self) -> Paddr {
        self.frames.into_raw().start
    }

    /// Restores ownership of a forgotten node.
    ///
    /// # Safety
    ///
    /// `paddr` must be the base address of a forgotten node of configuration `C`.
    pub(super) unsafe fn from_raw(paddr: Paddr) -> Self {
        // SAFETY: The caller guarantees that the node is alive and has type `C`.
        let node = unsafe { PageTableNodeRef::<C>::borrow_paddr(paddr) };
        let size = page_table_node_size::<C>(node.level());
        Self {
            // SAFETY: All frames were forgotten together, and the level determines
            // the exact size of the original allocation.
            frames: unsafe { Segment::from_raw(paddr..paddr + size) },
        }
    }

    /// Allocates a new empty page table node.
    pub(super) fn alloc(level: PagingLevel) -> Self {
        const {
            assert!(C::PTE_SIZE == size_of::<C::E>());
            assert!(C::PTE_SIZE.is_power_of_two() && C::PTE_SIZE <= PAGE_SIZE);
        }
        let size = page_table_node_size::<C>(level);
        let frames = FrameAllocOptions::new()
            .alignment(size)
            .alloc_segment_with(size / PAGE_SIZE, |_| PageTablePageMeta::new(level))
            .expect("Failed to allocate a page table node");
        Self { frames }
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
    pub(super) unsafe fn activate(&self) {
        use crate::arch::mm::{activate_page_table, current_page_table_paddr};

        assert_eq!(self.level(), nr_levels::<C>());

        let last_activated_paddr = current_page_table_paddr();
        if last_activated_paddr == self.paddr() {
            return;
        }

        // SAFETY: The safety is upheld by the caller.
        unsafe { activate_page_table(self.clone().into_raw()) };

        // Restore and drop the last activated page table.
        // SAFETY: The physical address is valid and points to a forgotten page table node.
        drop(unsafe { Self::from_raw(last_activated_paddr) });
    }

    /// Activates the (root) page table assuming it is the first activation.
    ///
    /// It will not try dropping the last activate page table. It is the same
    /// with [`Self::activate()`] in other senses.
    pub(super) unsafe fn first_activate(&self) {
        use crate::arch::mm::activate_page_table;

        // SAFETY: The safety is upheld by the caller.
        unsafe { activate_page_table(self.clone().into_raw()) };
    }
}

/// A borrowed view of an entire page table node.
#[derive(Debug)]
pub(super) struct PageTableNodeRef<'a, C: PageTableConfig> {
    paddr: Paddr,
    level: PagingLevel,
    _marker: PhantomData<&'a PageTableNode<C>>,
}

impl<C: PageTableConfig> HasPaddr for PageTableNodeRef<'_, C> {
    fn paddr(&self) -> Paddr {
        self.paddr
    }
}

impl<'a, C: PageTableConfig> PageTableNodeRef<'a, C> {
    /// Borrows a complete node by its base physical address.
    ///
    /// # Safety
    ///
    /// `paddr` must point to the first frame of a node of configuration `C`,
    /// and all frames in that node must outlive the returned reference.
    pub(super) unsafe fn borrow_paddr(paddr: Paddr) -> Self {
        // Every backing frame records the same immutable level. Recover it
        // from the first frame when constructing the node-wide view.
        // SAFETY: The caller guarantees the metadata type and lifetime of
        // every frame in the node, including its first frame.
        let first_frame = unsafe { FrameRef::<PageTablePageMeta<C>>::borrow_paddr(paddr) };
        Self {
            paddr,
            level: first_frame.meta().level,
            _marker: PhantomData,
        }
    }

    pub(super) fn level(&self) -> PagingLevel {
        self.level
    }

    /// Borrows a backing frame. All frames have the node reference's lifetime.
    fn frame(&self, idx: usize) -> FrameRef<'a, PageTablePageMeta<C>> {
        assert!(idx < page_table_node_size::<C>(self.level()) / PAGE_SIZE);
        // SAFETY: The node reference keeps every backing frame alive with this metadata type.
        unsafe { FrameRef::borrow_paddr(self.paddr() + idx * PAGE_SIZE) }
    }

    /// Converts a nonempty entry range to the range of backing pages that cover it.
    fn pages_for_entries(&self, entries: Range<usize>) -> Range<usize> {
        assert!(entries.start < entries.end && entries.end <= nr_ptes::<C>(self.level()));
        let entries_per_page = PAGE_SIZE / C::PTE_SIZE;
        entries.start / entries_per_page..entries.end.div_ceil(entries_per_page)
    }

    /// Locks the backing pages covering `entries`, in increasing page order.
    ///
    /// # Panics
    ///
    /// Panics if `entries` is empty or outside the node.
    pub(super) fn lock<'rcu>(
        self,
        _guard: &'rcu dyn InAtomicMode,
        entries: Range<usize>,
    ) -> PageTableGuard<'rcu, C>
    where
        'a: 'rcu,
    {
        let pages = self.pages_for_entries(entries);
        // Record acquired locks as we go, so unwinding releases only those locks.
        let mut guard = PageTableGuard {
            inner: self,
            locked_pages: pages.start..pages.start,
        };
        for page in pages {
            let frame = guard.inner.frame(page);
            while frame
                .meta()
                .lock
                .compare_exchange(0, 1, Ordering::Acquire, Ordering::Relaxed)
                .is_err()
            {
                core::hint::spin_loop();
            }
            guard.locked_pages.end += 1;
        }
        guard
    }

    /// Restores a forgotten guard for the backing pages covering `entries`.
    ///
    /// # Safety
    ///
    /// The caller must own every page lock covering `entries`, and there must
    /// be no other guards for those pages. The range must cover exactly the
    /// pages whose lock ownership is being transferred to the returned guard.
    pub(super) unsafe fn make_guard_unchecked<'rcu>(
        self,
        _guard: &'rcu dyn InAtomicMode,
        entries: Range<usize>,
    ) -> PageTableGuard<'rcu, C>
    where
        'a: 'rcu,
    {
        let locked_pages = self.pages_for_entries(entries);
        PageTableGuard {
            inner: self,
            locked_pages,
        }
    }
}

/// Exclusive access to a contiguous range of backing pages in a node.
#[derive(Debug)]
pub(super) struct PageTableGuard<'rcu, C: PageTableConfig> {
    inner: PageTableNodeRef<'rcu, C>,
    locked_pages: Range<usize>,
}

impl<'rcu, C: PageTableConfig> PageTableGuard<'rcu, C> {
    /// Borrows an entry in the node at a given index.
    ///
    /// # Panics
    ///
    /// Panics if the index is not in the guard's locked entry range.
    pub(super) fn entry(&mut self, idx: usize) -> Entry<'_, 'rcu, C> {
        assert!(self.locked_entries().contains(&idx));
        // SAFETY: The index is within the bound.
        unsafe { Entry::new_at(self, idx) }
    }

    /// Returns the entry range protected by this guard, including page rounding.
    pub(super) fn locked_entries(&self) -> Range<usize> {
        let entries_per_page = PAGE_SIZE / C::PTE_SIZE;
        self.locked_pages.start * entries_per_page
            ..(self.locked_pages.end * entries_per_page).min(nr_ptes::<C>(self.level()))
    }

    /// Gets the number of valid PTEs in the locked backing pages.
    pub(super) fn nr_children(&self) -> usize {
        self.locked_pages
            .clone()
            .map(|page| {
                let frame = self.inner.frame(page);
                // SAFETY: This guard holds this frame's lock.
                unsafe { *frame.meta().nr_children.get() }
            })
            .sum()
    }

    /// Returns whether the node has been detached from its parent.
    pub(super) fn is_stray(&self) -> bool {
        let frame = self.inner.frame(self.locked_pages.start);
        // SAFETY: Detachment sets every flag while holding all page locks.
        // We hold this page's lock, so observing any one flag is sufficient.
        unsafe { *frame.meta().stray.get() }
    }

    /// Marks the node detached. All backing pages must be locked.
    pub(super) fn mark_stray(&mut self) {
        assert_eq!(self.locked_entries(), 0..nr_ptes::<C>(self.level()));
        for page in self.locked_pages.clone() {
            let frame = self.inner.frame(page);
            // SAFETY: This guard holds every page lock in the node.
            unsafe { *frame.meta().stray.get() = true };
        }
    }

    /// Reads a non-owning PTE at the given index.
    ///
    /// A non-owning PTE means that it does not account for a reference count
    /// of the a page if the PTE points to a page. The original PTE still owns
    /// the child page.
    ///
    /// # Safety
    ///
    /// The index must be within this guard's locked entry range.
    pub(super) unsafe fn read_pte(&self, idx: usize) -> C::E {
        debug_assert!(self.locked_entries().contains(&idx));
        let ptr = paddr_to_vaddr(self.paddr()) as *mut C::E;
        // SAFETY:
        // - The page table node is alive. The index is inside the bound, so the page table entry is valid.
        // - All page table entries are aligned and accessed with atomic operations only.
        unsafe { load_pte(ptr.add(idx), Ordering::Relaxed) }
    }

    /// Writes a page table entry at a given index.
    ///
    /// This operation will leak the old child if the old PTE is present.
    ///
    /// # Safety
    ///
    /// The caller must ensure that:
    ///  1. The index must be within this guard's locked entry range;
    ///  2. The PTE must represent a [`PteState`] in the same [`PageTableConfig`]
    ///     and at the right paging level (`self.level() - 1`).
    ///  3. The page table node will have the ownership of the [`PteState`]
    ///     after this method.
    pub(super) unsafe fn write_pte(&mut self, idx: usize, pte: C::E) {
        debug_assert!(self.locked_entries().contains(&idx));
        let ptr = paddr_to_vaddr(self.paddr()) as *mut C::E;
        // SAFETY:
        // - The page table node is alive. The index is inside the bound, so the page table entry is valid.
        // - All page table entries are aligned and accessed with atomic operations only.
        unsafe { store_pte(ptr.add(idx), pte, Ordering::Release) }
    }

    /// Updates the child count of the backing page containing this entry.
    fn update_nr_children(&mut self, idx: usize, op: impl Fn(&mut usize)) {
        debug_assert!(self.locked_entries().contains(&idx));
        let frame = self.inner.frame(idx / (PAGE_SIZE / C::PTE_SIZE));
        // SAFETY: This guard holds the containing frame's lock.
        op(unsafe { &mut *frame.meta().nr_children.get() });
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
        for page in self.locked_pages.clone().rev() {
            self.inner
                .frame(page)
                .meta()
                .lock
                .store(0, Ordering::Release);
        }
    }
}

/// The metadata of any kinds of page table pages.
/// Make sure the the generic parameters don't effect the memory layout.
#[derive(Debug)]
pub(crate) struct PageTablePageMeta<C: PageTableConfig> {
    /// The number of valid PTEs in this frame, protected by this frame's lock.
    nr_children: SyncUnsafeCell<usize>,
    /// Whether the node is detached from its parent, protected by this frame's lock.
    ///
    /// A page table can be detached from its parent while still being accessed,
    /// since we use a RCU scheme to recycle page tables. If this flag is set,
    /// it means that the parent is recycling the page table.
    stray: SyncUnsafeCell<bool>,
    /// The level of the page table page. A page table page cannot be
    /// referenced by page tables of different levels.
    level: PagingLevel,
    /// The lock for the page table page.
    lock: AtomicU8,
    _phantom: PhantomData<C>,
}

impl<C: PageTableConfig> PageTablePageMeta<C> {
    fn new(level: PagingLevel) -> Self {
        Self {
            nr_children: SyncUnsafeCell::new(0),
            stray: SyncUnsafeCell::new(false),
            level,
            lock: AtomicU8::new(0),
            _phantom: PhantomData,
        }
    }
}

// FIXME: The safe APIs in the `page_table/node` module allow `PteState::Frame`s with
// arbitrary addresses to be stored in the page table nodes. Therefore, they may not
// be valid `C::Item`s. The soundness of the following `on_drop` implementation must
// be reasoned in conjunction with the `page_table/cursor` implementation.
unsafe impl<C: PageTableConfig> AnyFrameMeta for PageTablePageMeta<C> {
    fn on_drop(&mut self, reader: &mut VmReader<Infallible>) {
        if *self.nr_children.get_mut() == 0 {
            return;
        }

        let level = self.level;
        let range = if level == nr_levels::<C>() {
            C::TOP_LEVEL_INDEX_RANGE.clone()
        } else {
            0..nr_ptes::<C>(level)
        };

        // The reader starts at this frame's base in the linear mapping. Nodes
        // are aligned to their size, so the physical address determines this
        // frame's offset.
        let frame_paddr = reader.cursor() as usize - LINEAR_MAPPING_BASE_VADDR;
        let first_pte = (frame_paddr % page_table_node_size::<C>(level)) / C::PTE_SIZE;

        let start = range.start.max(first_pte);
        let end = range.end.min(first_pte + PAGE_SIZE / C::PTE_SIZE);
        if start >= end {
            return;
        }
        reader.skip((start - first_pte) * C::PTE_SIZE);

        for _ in start..end {
            // Non-atomic read is OK because we have mutable access.
            let pte = reader.read_once::<C::E>().unwrap();
            match pte.to_repr(level) {
                PteScalar::PageTable(child_pt_addr, _) => {
                    // SAFETY: The PTE points to a page table node. The ownership
                    // of the child is transferred to the child then dropped.
                    drop(unsafe { PageTableNode::<C>::from_raw(child_pt_addr) });
                }
                PteScalar::Mapped(pa, prop) => {
                    // SAFETY: The PTE points to a mapped item. The ownership
                    // of the item is transferred here then dropped.
                    drop(unsafe { C::item_from_raw(pa, level, prop) });
                }
                PteScalar::Absent => {}
            }
        }
    }
}
