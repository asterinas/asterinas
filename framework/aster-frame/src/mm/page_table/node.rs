// SPDX-License-Identifier: MPL-2.0

//! Page table nodes and handles.
//!
//! The page table node is also frequently referred to as a page table in many architectural
//! documentations. It is essentially a page that contains page table entries (PTEs) that map
//! to child page tables nodes or mapped pages.
//!
//! This module leverages the frame metadata to manage the page table frames, which makes it
//! easier to provide the following guarantees:
//!
//! The page table node is not freed when it is still in use by:
//!    - a parent page table node,
//!    - or a handle to a page table node,
//!    - or a processor.
//! This is implemented by using a reference counter in the frame metadata. If the above
//! conditions are not met, the page table node is ensured to be freed upon dropping the last
//! reference.
//!
//! One can acquire exclusive access to a page table node using merely the physical address of
//! the page table node. This is implemented by a lock in the frame metadata. Here the
//! exclusiveness is only ensured for kernel code, and the processor's MMU is able to access the
//! page table node while a lock is held. So the modification to the PTEs should be done after
//! the initialization of the entity that the PTE points to. This is taken care in this module.
//!
//! # Safety
//!
//! This module is primarily concerned with the memory safety of the page tables themselves. That
//! is, by treating page table nodes as a special kind of container, we ensure the memory safety of
//! the containers without worrying about the validity of the data they contain.
//!
//! For example, if a PTE points to another page table, it is necessary to ensure that it points to
//! a valid page table. Otherwise, it breaks the memory safety of the page tables themselves.
//!
//! However, if a PTE points to a physical page, it can point to an arbitrary page (e.g., by
//! calling a safe [`PageTableNode::set_child_untracked`] method). This will never break the memory
//! safety of the page table itself.
//!
//! Users will need additional unsafe code to make further assumptions about the validity of the
//! page table contents. For example, if users want to activate a page table, they need to call the
//! unsafe method [`RootPageTableNode::activate`]; if users want to assume that the page table
//! entry points to a tracked frame (instead of arbitrary regions of memory), they need to call
//! [`MaybeTracked::assume_tracked`].

use core::{
    marker::PhantomData,
    mem::ManuallyDrop,
    ops::{Deref, DerefMut, Range},
    panic,
    sync::atomic::Ordering,
};

use super::{nr_subpage_per_huge, page_size, PageTableEntryTrait};
use crate::{
    arch::mm::{PageTableEntry, PagingConsts},
    mm::{
        paddr_to_vaddr,
        page::{
            allocator::FRAME_ALLOCATOR,
            meta::{FrameMeta, PageMeta, PageTablePageMeta, PageTablePageMetaInner, PageUsage},
            Page,
        },
        page_prop::PageProperty,
        Frame, Paddr, PagingConstsTrait, PagingLevel, PAGE_SIZE,
    },
    task::{disable_preempt, DisablePreemptGuard},
};

/// The raw handle to a page table node.
///
/// This handle is a referencer of a page table node. Thus creating and dropping it will affect
/// the reference count of the page table node. If dropped the raw handle as the last reference,
/// the page table node and subsequent children will be freed.
///
/// Only the CPU or a PTE can access a page table node using a raw handle. To access the page
/// table frame from the kernel code, use the handle [`PageTableNode`].
#[derive(Debug)]
pub(super) struct RawPageTableNode<E: PageTableEntryTrait, C: PagingConstsTrait>
where
    [(); C::NR_LEVELS as usize]:,
{
    pub(super) raw: Paddr,
    _phantom: PhantomData<(E, C)>,
}

impl<E: PageTableEntryTrait, C: PagingConstsTrait> RawPageTableNode<E, C>
where
    [(); C::NR_LEVELS as usize]:,
{
    pub(super) fn paddr(&self) -> Paddr {
        self.raw
    }

    /// Convert a raw handle to an accessible handle by pertaining the lock.
    pub(super) fn lock(self) -> PageTableNode<E, C> {
        // Prevent dropping the handle.
        let this = ManuallyDrop::new(self);

        // SAFETY: The physical address in the raw handle is valid and we are
        // transferring the ownership to a new handle. No increment of the reference
        // count is needed.
        let page = unsafe { Page::<PageTablePageMeta<E, C>>::from_raw(this.paddr()) };

        let disable_preempt = disable_preempt();

        // Acquire the lock.
        while page
            .meta()
            .lock
            .compare_exchange(0, 1, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            core::hint::spin_loop();
        }

        PageTableNode::<E, C> {
            page,
            preempt_guard: disable_preempt,
        }
    }

    /// Create a copy of the handle.
    pub(super) fn clone_shallow(&self) -> Self {
        self.increment_strong_count();

        Self {
            raw: self.raw,
            _phantom: PhantomData,
        }
    }

    fn increment_strong_count(&self) {
        // SAFETY: We have a reference count to the page and can safely increase the reference
        // count by one more.
        unsafe {
            Page::<PageTablePageMeta<E, C>>::increment_strong_count(self.paddr());
        }
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
/// table frame has no references. You can set the page table node as a child of another
/// page table node.
#[derive(Debug)]
pub(super) struct PageTableNode<
    E: PageTableEntryTrait = PageTableEntry,
    C: PagingConstsTrait = PagingConsts,
> where
    [(); C::NR_LEVELS as usize]:,
{
    pub(super) page: Page<PageTablePageMeta<E, C>>,
    preempt_guard: DisablePreemptGuard,
}

/// A tracked frame or an untracked physical memory region.
pub(super) trait MaybeTracked {
    /// Assumes that there is an untracked physical memory, returning the start physical address
    /// and the page property.
    ///
    /// Calling this method on tracked frames alone does not break memory safety, but it can cause
    /// resource leakage.
    fn assume_untracked(self) -> (Paddr, PageProperty);

    /// Assumes that there is a tracked frame, returning the frame and the page property.
    ///
    /// Note that if `self` does not have ownership of the tracked frame, this method should clone
    /// the frame and return the cloned frame. Otherwise, ownership of the frame will be
    /// transferred to the returned frame.
    ///
    /// # Safety
    ///
    /// The caller must ensure that a child frame has been previously set in the page table (with
    /// [PageTableNode::set_child_frame]) and has not been overwritten or split.
    ///
    /// The page table itself does not maintain any information about whether its entries point to
    /// tracked frames or untracked regions of memory, so it is up to the caller to ensure that the
    /// above assumption actually holds.
    unsafe fn assume_tracked(self) -> (Frame, PageProperty);
}

/// A page pointed by a page table entry, representing either a tracked frame or an untracked
/// physical memory region.
///
/// An instance takes ownership of the underlying frame, so it may cause resource leakage or panic
/// if it is dropped without knowning whether it represents a tracked frame or an untracked
/// physical memory region.
#[derive(Debug)]
pub(super) struct MaybeTrackedPage {
    paddr: Paddr,
    prop: PageProperty,
}

impl MaybeTracked for MaybeTrackedPage {
    fn assume_untracked(self) -> (Paddr, PageProperty) {
        let this = ManuallyDrop::new(self);

        (this.paddr, this.prop)
    }

    unsafe fn assume_tracked(self) -> (Frame, PageProperty) {
        let this = ManuallyDrop::new(self);

        (
            Frame {
                page: unsafe { Page::<FrameMeta>::from_raw(this.paddr) },
            },
            this.prop,
        )
    }
}

impl Drop for MaybeTrackedPage {
    fn drop(&mut self) {
        debug_assert!(
            false,
            "cannot drop `MaybeTrackedPage` without knowing whether the page is tracked"
        );
    }
}

/// A reference to a page pointed by a page table entry, representing either a tracked frame or an
/// untracked physical memory region.
///
/// An instance does not take ownership of the underlying frame, so it can be dropped silently.
#[derive(Debug)]
pub(super) struct MaybeTrackedPageRef<'a> {
    paddr: Paddr,
    prop: PageProperty,
    _phantom: PhantomData<&'a Frame>,
}

impl<'a> MaybeTrackedPageRef<'a> {
    pub(super) fn prop(&self) -> PageProperty {
        self.prop
    }
}

impl<'a> MaybeTracked for MaybeTrackedPageRef<'a> {
    fn assume_untracked(self) -> (Paddr, PageProperty) {
        let paddr = self.paddr;
        let prop = self.prop;
        (paddr, prop)
    }

    unsafe fn assume_tracked(self) -> (Frame, PageProperty) {
        let frame = Frame {
            page: unsafe { Page::<FrameMeta>::from_raw(self.paddr) },
        };
        let prop = self.prop;
        core::mem::forget(frame.clone());
        (frame, prop)
    }
}

/// A child of a page table node.
///
/// A child will point to either a page, a page table, or nothing.
///
/// When a child takes ownership (i.e., `T = MaybeTrackedPage`), either
/// [`Child::drop_deep_tracked`], [`Child::drop_deep_untracked`], or [`Child::drop_none`] should be
/// called to avoid restore leakage.
///
/// However, if a child takes a reference (i.e., `T = MaybeTrackedPageRef`), it is fine to silently
/// drop the child.
#[must_use]
#[derive(Debug)]
pub(super) enum Child<
    T,
    E: PageTableEntryTrait = PageTableEntry,
    C: PagingConstsTrait = PagingConsts,
> where
    [(); C::NR_LEVELS as usize]:,
{
    Page(T),
    PageTable(RawPageTableNode<E, C>),
    None,
}

impl<T, E: PageTableEntryTrait, C: PagingConstsTrait> Child<T, E, C>
where
    [(); C::NR_LEVELS as usize]:,
{
    pub(super) fn is_none(&self) -> bool {
        matches!(self, Self::None)
    }
}

impl<E: PageTableEntryTrait, C: PagingConstsTrait> Child<MaybeTrackedPage, E, C>
where
    [(); C::NR_LEVELS as usize]:,
{
    pub(super) fn drop_deep_untracked(self) {
        let mut pt = match self {
            Self::Page(maybe_tracked) => {
                let _ = maybe_tracked.assume_untracked();
                return;
            }
            Self::PageTable(table) => table.lock(),
            Self::None => return,
        };

        // Fast path
        if pt.meta().nr_children == 0 {
            return;
        }

        for i in 0..nr_subpage_per_huge::<C>() {
            pt.unset_child(i).drop_deep_untracked();
        }
    }

    pub(super) unsafe fn drop_deep_tracked(self) {
        let mut pt = match self {
            Self::Page(maybe_tracked) => {
                drop(unsafe { maybe_tracked.assume_tracked() });
                return;
            }
            Self::PageTable(table) => table.lock(),
            Self::None => return,
        };

        // Fast path
        if pt.meta().nr_children == 0 {
            return;
        }

        for i in 0..nr_subpage_per_huge::<C>() {
            unsafe {
                pt.unset_child(i).drop_deep_tracked();
            }
        }
    }

    pub(super) fn drop_none(self) {
        debug_assert!(matches!(self, Self::None));
    }
}

impl<E: PageTableEntryTrait, C: PagingConstsTrait> Child<MaybeTrackedPage, E, C>
where
    [(); C::NR_LEVELS as usize]:,
{
    /// Takes ownership of the PTE and converts to a child.
    ///
    /// # Safety
    ///
    /// If the PTE points to a page table, the caller must ensure that it points to a valid page
    /// table.
    unsafe fn from_pte(pte: E, level: PagingLevel) -> Self {
        if !pte.is_present() {
            Self::None
        } else if pte.is_last(level) {
            Self::Page(MaybeTrackedPage {
                paddr: pte.paddr(),
                prop: pte.prop(),
            })
        } else {
            Self::PageTable(RawPageTableNode {
                raw: pte.paddr(),
                _phantom: PhantomData,
            })
        }
    }
}

impl<E: PageTableEntryTrait, C: PagingConstsTrait> PageTableNode<E, C>
where
    [(); C::NR_LEVELS as usize]:,
{
    /// Allocate a new empty page table node.
    ///
    /// This function returns an owning handle. The newly created handle does not
    /// set the lock bit for performance as it is exclusive and unlocking is an
    /// extra unnecessary expensive operation.
    pub(super) fn alloc(level: PagingLevel) -> Self {
        let frame = FRAME_ALLOCATOR.get().unwrap().lock().alloc(1).unwrap() * PAGE_SIZE;
        let page = Page::<PageTablePageMeta<E, C>>::from_unused(frame);

        // The lock is initialized as held.
        page.meta().lock.store(1, Ordering::Relaxed);

        // SAFETY: here the page exclusively owned by the newly created handle.
        let inner = unsafe { &mut *page.meta().inner.get() };
        inner.level = level;

        // Zero out the page table node.
        let ptr = paddr_to_vaddr(page.paddr()) as *mut u8;
        // SAFETY: The page is exclusively owned here. Pointers are valid also.
        // We rely on the fact that 0 represents an absent entry to speed up `memset`.
        unsafe { core::ptr::write_bytes(ptr, 0, PAGE_SIZE) };
        debug_assert!(E::new_absent().as_bytes().iter().all(|&b| b == 0));

        Self {
            page,
            preempt_guard: disable_preempt(),
        }
    }

    pub fn level(&self) -> PagingLevel {
        self.meta().level
    }

    /// Convert the handle into a raw handle to be stored in a PTE or CPU.
    pub(super) fn into_raw(self) -> RawPageTableNode<E, C> {
        let mut this = ManuallyDrop::new(self);

        let raw = this.page.paddr();

        this.page.meta().lock.store(0, Ordering::Release);
        // SAFETY: The field will no longer be accessed and we need to drop the field to release
        // the preempt count.
        unsafe {
            core::ptr::drop_in_place(&mut this.preempt_guard);
        }

        RawPageTableNode {
            raw,
            _phantom: PhantomData,
        }
    }

    /// Get a raw handle while still preserving the original handle.
    pub(super) fn clone_raw(&self) -> RawPageTableNode<E, C> {
        core::mem::forget(self.page.clone());

        RawPageTableNode {
            raw: self.page.paddr(),
            _phantom: PhantomData,
        }
    }

    /// Get an extra reference of the child at the given index.
    pub(super) fn child(&self, idx: usize) -> Child<MaybeTrackedPageRef<'_>, E, C> {
        let pte = self.read_pte(idx);
        if !pte.is_present() {
            return Child::None;
        }

        let paddr = pte.paddr();
        if pte.is_last(self.level()) {
            return Child::Page(MaybeTrackedPageRef {
                paddr,
                prop: pte.prop(),
                _phantom: PhantomData,
            });
        }

        unsafe {
            Page::<PageTablePageMeta<E, C>>::increment_strong_count(paddr);
        }
        Child::PageTable(RawPageTableNode {
            raw: paddr,
            _phantom: PhantomData,
        })
    }

    /// Remove a child if the child at the given index is present.
    pub(super) fn unset_child(&mut self, idx: usize) -> Child<MaybeTrackedPage, E, C> {
        assert!(idx < nr_subpage_per_huge::<C>());

        // SAFETY: The PTE does not point to a page table.
        unsafe { self.overwrite_pte(idx, None) }
    }

    /// Set a child page table at a given index.
    pub(super) fn set_child_pt(
        &mut self,
        idx: usize,
        pt: RawPageTableNode<E, C>,
    ) -> Child<MaybeTrackedPage, E, C> {
        assert!(self.level() > 1);

        // The ownership is transferred to a raw PTE. Don't drop the handle.
        let pt = ManuallyDrop::new(pt);

        let pte = Some(E::new_pt(pt.paddr()));

        // SAFETY: The PTE points to a valid page table. The page table level is checked above to
        // make sure the PTE can be correctly interpreted as a page table.
        //
        // There is still the possibility that the level of the given page table is incorrect,
        // which breaks the structure of the page table, but does not affect the memory safety of
        // the page table itself.
        unsafe { self.overwrite_pte(idx, pte) }
    }

    /// Map a frame at a given index.
    pub(super) fn set_child_frame(
        &mut self,
        idx: usize,
        frame: Frame,
        prop: PageProperty,
    ) -> Child<MaybeTrackedPage, E, C> {
        debug_assert_eq!(frame.level(), self.level());

        // The ownership is transferred to a raw PTE. Don't drop the handle.
        let frame = ManuallyDrop::new(frame);

        let pte = Some(E::new_frame(frame.start_paddr(), self.level(), prop));

        // SAFETY: The PTE does not point to a page table.
        unsafe { self.overwrite_pte(idx, pte) }
    }

    /// Set an untracked child frame at a given index.
    pub(super) fn set_child_untracked(
        &mut self,
        idx: usize,
        pa: Paddr,
        prop: PageProperty,
    ) -> Child<MaybeTrackedPage, E, C> {
        let pte = Some(E::new_frame(pa, self.level(), prop));

        // SAFETY: The PTE does not point to a page table.
        unsafe { self.overwrite_pte(idx, pte) }
    }

    /// Split the untracked huge page mapped at `idx` to smaller pages.
    pub(super) fn split_untracked_huge(&mut self, idx: usize) {
        let Child::Page(maybe_tracked) = self.child(idx) else {
            panic!("`split_untracked_huge` not called on an untracked huge page");
        };
        let (pa, prop) = maybe_tracked.assume_untracked();

        let mut new_frame = PageTableNode::<E, C>::alloc(self.level() - 1);
        for i in 0..nr_subpage_per_huge::<C>() {
            let small_pa = pa + i * page_size::<C>(self.level() - 1);
            new_frame.set_child_untracked(i, small_pa, prop).drop_none();
        }

        self.set_child_pt(idx, new_frame.into_raw())
            .drop_deep_untracked();
    }

    /// Protect an already mapped child at a given index.
    pub(super) fn protect(&mut self, idx: usize, prop: PageProperty) {
        let mut pte = self.read_pte(idx);
        debug_assert!(pte.is_present());

        pte.set_prop(prop);

        // SAFETY: The index is in bounds as checked by `read_pte`. The PTE is still valid as only
        // the page property is changed.
        unsafe {
            (self.as_ptr() as *mut E).add(idx).write(pte);
        }
    }

    fn read_pte(&self, idx: usize) -> E {
        assert!(idx < nr_subpage_per_huge::<C>());

        // SAFETY: The index is in bounds as checked above.
        unsafe { self.as_ptr().add(idx).read() }
    }

    fn start_paddr(&self) -> Paddr {
        self.page.paddr()
    }

    /// Replaces a page table entry at a given index.
    ///
    /// The ownership of the replaced PTE is transferred to the return value. If the replaced PTE
    /// points to a page, the method does not know whether the page represents a tracked frame or
    /// an untracked region of memory. It's up to the caller to make further calls to
    /// [`Child::drop_deep_tracked`], [`Child::drop_deep_tracked`], or [`Child::drop_none`] to drop
    /// the child.
    ///
    /// # Safety
    ///
    /// If the PTE points to a page table, the caller must ensure that it must point to a valid
    /// page table.
    unsafe fn overwrite_pte(
        &mut self,
        idx: usize,
        pte: Option<E>,
    ) -> Child<MaybeTrackedPage, E, C> {
        let existing_pte = self.read_pte(idx);

        // SAFETY: The index is in bounds as checked by `read_pte`. The PTE validity is upheld by
        // the caller.
        unsafe {
            (self.as_ptr() as *mut E)
                .add(idx)
                .write(pte.unwrap_or(E::new_absent()))
        };

        // Update the child count.
        if existing_pte.is_present() && pte.is_none() {
            self.meta_mut().nr_children -= 1;
        } else if !existing_pte.is_present() && pte.is_some() {
            self.meta_mut().nr_children += 1;
        }

        unsafe { Child::from_pte(existing_pte, self.level()) }
    }

    fn as_ptr(&self) -> *const E {
        paddr_to_vaddr(self.start_paddr()) as *const E
    }

    fn meta(&self) -> &PageTablePageMetaInner {
        // SAFETY: Here we have an exclusive access to the page.
        unsafe { &*self.page.meta().inner.get() }
    }

    fn meta_mut(&mut self) -> &mut PageTablePageMetaInner {
        // SAFETY: Here we have an exclusive access to the page.
        unsafe { &mut *self.page.meta().inner.get() }
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

unsafe impl<E: PageTableEntryTrait, C: PagingConstsTrait> PageMeta for PageTablePageMeta<E, C>
where
    [(); C::NR_LEVELS as usize]:,
{
    const USAGE: PageUsage = PageUsage::PageTable;

    fn on_drop(page: &mut Page<Self>) {
        let paddr = page.paddr();

        let inner = unsafe { &mut *page.meta().inner.get() };
        let level = inner.level;

        // Drop the children.
        for i in 0..nr_subpage_per_huge::<C>() {
            // SAFETY: The index is within the bound and PTE is plain-old-data. The
            // address is aligned as well. We also have an exclusive access ensured
            // by reference counting.
            let pte_ptr = unsafe { (paddr_to_vaddr(paddr) as *const E).add(i) };
            // SAFETY: The pointer is valid and the PTE is plain-old-data.
            let pte = unsafe { pte_ptr.read() };

            // Note that if the PTE points to a physical page, we have no way of knowing whether it
            // points to a tracked frame or an untracked region of memory at that point. In this
            // case it will cause panic or resource leakage (see the implementation of
            // `MaybeTrackedPage::drop`).
            //
            // SAFETY: We can consume ownership of the PTE because we are dropping the page table.
            drop(unsafe { Child::from_pte(pte, level) });
        }

        // Recycle this page table node.
        FRAME_ALLOCATOR
            .get()
            .unwrap()
            .lock()
            .dealloc(paddr / PAGE_SIZE, 1);
    }
}

/// Most methods of [`PageTableNode`] do not make any assumption of the page table contents. But
/// here are some exceptions.
impl<E: PageTableEntryTrait, C: PagingConstsTrait> PageTableNode<E, C>
where
    [(); C::NR_LEVELS as usize]:,
{
    /// Make a deep copy of the child at the given index.
    ///
    /// # Safety
    ///
    /// The caller must ensure that the child and all its descendants contain only tracked frames
    /// and no untracked memory regions.
    unsafe fn deep_copy_child_from(&mut self, idx: usize, src: &PageTableNode<E, C>) {
        match src.child(idx) {
            Child::PageTable(pt) => {
                let guard = pt.clone_shallow().lock();
                // SAFETY: The safety is upheld by the caller.
                let new_child = unsafe { guard.deep_copy() };
                self.set_child_pt(idx, new_child.into_raw()).drop_none();
            }
            Child::Page(maybe_tracked) => {
                // SAFETY: The safety is upheld by the caller.
                let (frame, prop) = unsafe { maybe_tracked.assume_tracked() };
                self.set_child_frame(idx, frame, prop).drop_none();
            }
            Child::None => (),
        }
    }

    /// Makes a deep copy.
    ///
    /// # Safety
    ///
    /// The caller must ensure that the page table and all its descendants contain only tracked
    /// frames and no untracked memory regions.
    unsafe fn deep_copy(&self) -> Self {
        let mut new_frame = Self::alloc(self.level());

        for i in 0..nr_subpage_per_huge::<C>() {
            // SAFETY: The safety is upheld by the caller.
            unsafe {
                new_frame.deep_copy_child_from(i, self);
            }
        }

        new_frame
    }
}

/// A root page table node.
#[derive(Debug)]
pub(super) struct RootPageTableNode<E: PageTableEntryTrait, C: PagingConstsTrait>(
    RawPageTableNode<E, C>,
)
where
    [(); C::NR_LEVELS as usize]:;

impl<E: PageTableEntryTrait, C: PagingConstsTrait> Deref for RootPageTableNode<E, C>
where
    [(); C::NR_LEVELS as usize]:,
{
    type Target = RawPageTableNode<E, C>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<E: PageTableEntryTrait, C: PagingConstsTrait> DerefMut for RootPageTableNode<E, C>
where
    [(); C::NR_LEVELS as usize]:,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<E: PageTableEntryTrait, C: PagingConstsTrait> RootPageTableNode<E, C>
where
    [(); C::NR_LEVELS as usize]:,
{
    pub(super) fn alloc() -> Self {
        Self(PageTableNode::alloc(C::NR_LEVELS).into_raw())
    }

    /// Activate the page table assuming it is a root page table.
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

        activate_page_table(self.raw, CachePolicy::Writeback);

        if last_activated_paddr == self.raw {
            return;
        }

        // Increment the reference count of the current page table.
        self.increment_strong_count();

        // Restore and drop the last activated page table.
        drop(RawPageTableNode::<E, C> {
            raw: last_activated_paddr,
            _phantom: PhantomData,
        });
    }

    /// Activate the (root) page table assuming it is the first activation.
    ///
    /// It will not try dropping the last activate page table. It is the same
    /// with [`Self::activate()`] in other senses.
    pub(super) unsafe fn first_activate(&self) {
        use crate::{arch::mm::activate_page_table, mm::CachePolicy};

        self.increment_strong_count();

        activate_page_table(self.raw, CachePolicy::Writeback);
    }

    /// Makes a copy of the page table.
    ///
    /// This function allows the caller to control about the way to copy the children.
    /// For indexes in `deep`, the children are deeply copied in a recursive manner.
    /// For indexes in `shallow`, the children are shallowly copied as new references.
    ///
    /// # Panic
    ///
    /// The method will panic if the two ranges `deep` and `shallow` are not disjoint.
    ///
    /// The method will also panic if the children to be shallowly copied point to pages instead of
    /// page tables.
    ///
    /// # Safety
    ///
    /// The caller must ensure that the children to be deeply copied (recursively) contain only
    /// tracked frames and no untracked memory regions.
    pub(super) unsafe fn make_copy(&self, deep: Range<usize>, shallow: Range<usize>) -> Self {
        Self::make_copy_locked(&self.clone_shallow().lock(), deep, shallow)
    }

    /// Makes a copy of the page table.
    ///
    /// This method is similar to [`Self::make_copy`], but takes a locked page table to avoid race
    /// conditions.
    ///
    /// # Panic
    ///
    /// In addition to the scenarios described by [`Self::make_copy`], this method will
    /// panic if the given page table is not a root page table.
    ///
    /// # Safety
    ///
    /// The caller must ensure that the children to be deeply copied recursively contain only
    /// tracked frames and no untracked memory regions.
    pub(super) unsafe fn make_copy_locked(
        locked: &PageTableNode<E, C>,
        deep: Range<usize>,
        shallow: Range<usize>,
    ) -> Self {
        // This is a heavy method and shouldn't be called that often, so a few more checks like
        // this are just fine.
        assert!(deep.end <= shallow.start || deep.start >= shallow.end);
        assert!(locked.level() == C::NR_LEVELS);

        let mut new_frame = PageTableNode::alloc(C::NR_LEVELS);

        for i in deep {
            new_frame.deep_copy_child_from(i, locked);
        }

        for i in shallow {
            match locked.child(i) {
                Child::PageTable(pt) => {
                    new_frame.set_child_pt(i, pt.clone_shallow()).drop_none();
                }
                Child::Page(_) => {
                    panic!("cannot make shallow copies to pages")
                }
                Child::None => (),
            }
        }

        Self(new_frame.into_raw())
    }

    pub(super) fn clone_shallow_root(&self) -> Self {
        Self(self.0.clone_shallow())
    }
}
