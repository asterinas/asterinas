// SPDX-License-Identifier: MPL-2.0

//! This module defines page table node abstractions and the handle.
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

use core::{marker::PhantomData, mem::ManuallyDrop, ops::Range, panic, sync::atomic::Ordering};

use super::{nr_subpage_per_huge, page_size, PageTableEntryTrait};
use crate::{
    arch::mm::{PageTableEntry, PagingConsts},
    mm::{
        paddr_to_vaddr,
        page::{
            allocator::FRAME_ALLOCATOR,
            meta::{FrameMeta, PageMeta, PageTablePageMeta, PageUsage},
            Page,
        },
        page_prop::PageProperty,
        Frame, Paddr, PagingConstsTrait, PagingLevel, PAGE_SIZE,
    },
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
    pub(super) level: PagingLevel,
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
        // SAFETY: The physical address in the raw handle is valid and we are
        // transferring the ownership to a new handle. No increment of the reference
        // count is needed.
        let page = unsafe { Page::<PageTablePageMeta<E, C>>::from_raw(self.paddr()) };
        debug_assert!(page.meta().level == self.level);
        // Acquire the lock.
        while page
            .meta()
            .lock
            .compare_exchange(0, 1, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            core::hint::spin_loop();
        }
        // Prevent dropping the handle.
        let _ = ManuallyDrop::new(self);
        PageTableNode::<E, C> { page }
    }

    /// Create a copy of the handle.
    pub(super) fn clone_shallow(&self) -> Self {
        self.inc_ref();
        Self {
            raw: self.raw,
            level: self.level,
            _phantom: PhantomData,
        }
    }

    pub(super) fn nr_valid_children(&self) -> u16 {
        // SAFETY: The physical address in the raw handle is valid and we are
        // accessing the page table node. We forget the handle when finished.
        let page = unsafe { Page::<PageTablePageMeta<E, C>>::from_raw(self.paddr()) };
        let nr = page.meta().nr_children;
        core::mem::forget(page);
        nr
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
        use core::sync::atomic::AtomicBool;

        use crate::{
            arch::mm::{activate_page_table, current_page_table_paddr},
            mm::CachePolicy,
        };

        debug_assert_eq!(self.level, PagingConsts::NR_LEVELS);

        let last_activated_paddr = current_page_table_paddr();

        activate_page_table(self.raw, CachePolicy::Writeback);

        if last_activated_paddr == self.raw {
            return;
        }

        // Increment the reference count of the current page table.
        self.inc_ref();

        // Decrement the reference count of the last activated page table.

        // Boot page tables are not tracked with [`PageTableNode`], but
        // all page tables after the boot stage are tracked.
        //
        // TODO: the `cpu_local` implementation currently is underpowered,
        // there's no need using `AtomicBool` here.
        crate::cpu_local! {
            static CURRENT_IS_BOOT_PT: AtomicBool = AtomicBool::new(true);
        }
        if !CURRENT_IS_BOOT_PT.load(Ordering::Acquire) {
            // Restore and drop the last activated page table.
            let _last_activated_pt = Self {
                raw: last_activated_paddr,
                level: PagingConsts::NR_LEVELS,
                _phantom: PhantomData,
            };
        } else {
            CURRENT_IS_BOOT_PT.store(false, Ordering::Release);
        }
    }

    fn inc_ref(&self) {
        // SAFETY: The physical address in the raw handle is valid and we are
        // incrementing the reference count by cloning and forgetting.
        let page = unsafe { Page::<PageTablePageMeta<E, C>>::from_raw(self.paddr()) };
        core::mem::forget(page.clone());
        core::mem::forget(page);
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
}

/// A child of a page table node.
#[derive(Debug)]
pub(super) enum Child<E: PageTableEntryTrait = PageTableEntry, C: PagingConstsTrait = PagingConsts>
where
    [(); C::NR_LEVELS as usize]:,
{
    PageTable(RawPageTableNode<E, C>),
    Frame(Frame),
    /// Frames not tracked by handles.
    Untracked(Paddr),
    None,
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
        let mut page = Page::<PageTablePageMeta<E, C>>::from_unused(frame);
        // The lock is initialized as held.
        page.meta().lock.store(1, Ordering::Relaxed);

        // SAFETY: here the page exclusively owned by the newly created handle.
        unsafe { page.meta_mut().level = level };

        // Zero out the page table node.
        let ptr = paddr_to_vaddr(page.paddr()) as *mut u8;
        // SAFETY: The page is exclusively owned here. Pointers are valid also.
        // We rely on the fact that 0 represents an absent entry to speed up `memset`.
        unsafe { core::ptr::write_bytes(ptr, 0, PAGE_SIZE) };
        debug_assert!(E::new_absent().as_bytes().iter().all(|&b| b == 0));

        Self { page }
    }

    pub fn level(&self) -> PagingLevel {
        self.page.meta().level
    }

    /// Convert the handle into a raw handle to be stored in a PTE or CPU.
    pub(super) fn into_raw(self) -> RawPageTableNode<E, C> {
        let level = self.level();
        let raw = self.page.paddr();
        self.page.meta().lock.store(0, Ordering::Release);
        core::mem::forget(self);
        RawPageTableNode {
            raw,
            level,
            _phantom: PhantomData,
        }
    }

    /// Get a raw handle while still preserving the original handle.
    pub(super) fn clone_raw(&self) -> RawPageTableNode<E, C> {
        core::mem::forget(self.page.clone());
        RawPageTableNode {
            raw: self.page.paddr(),
            level: self.level(),
            _phantom: PhantomData,
        }
    }

    /// Get an extra reference of the child at the given index.
    pub(super) fn child(&self, idx: usize, tracked: bool) -> Child<E, C> {
        debug_assert!(idx < nr_subpage_per_huge::<C>());
        let pte = self.read_pte(idx);
        if !pte.is_present() {
            Child::None
        } else {
            let paddr = pte.paddr();
            if !pte.is_last(self.level()) {
                // SAFETY: The physical address is recorded in a valid PTE
                // which would be casted from a handle. We are incrementing
                // the reference count so we restore, clone, and forget both.
                let node = unsafe { Page::<PageTablePageMeta<E, C>>::from_raw(paddr) };
                let inc_ref = node.clone();
                core::mem::forget(node);
                core::mem::forget(inc_ref);
                Child::PageTable(RawPageTableNode {
                    raw: paddr,
                    level: self.level() - 1,
                    _phantom: PhantomData,
                })
            } else if tracked {
                // SAFETY: The physical address is recorded in a valid PTE
                // which would be casted from a handle. We are incrementing
                // the reference count so we restore and forget a cloned one.
                let page = unsafe { Page::<FrameMeta>::from_raw(paddr) };
                core::mem::forget(page.clone());
                Child::Frame(Frame { page })
            } else {
                Child::Untracked(paddr)
            }
        }
    }

    /// Make a copy of the page table node.
    ///
    /// This function allows you to control about the way to copy the children.
    /// For indexes in `deep`, the children are deep copied and this function will be recursively called.
    /// For indexes in `shallow`, the children are shallow copied as new references.
    ///
    /// You cannot shallow copy a child that is mapped to a frame. Deep copying a frame child will not
    /// copy the mapped frame but will copy the handle to the frame.
    ///
    /// You cannot either deep copy or shallow copy a child that is mapped to an untracked frame.
    ///
    /// The ranges must be disjoint.
    pub(super) unsafe fn make_copy(&self, deep: Range<usize>, shallow: Range<usize>) -> Self {
        let mut new_frame = Self::alloc(self.level());
        debug_assert!(deep.end <= nr_subpage_per_huge::<C>());
        debug_assert!(shallow.end <= nr_subpage_per_huge::<C>());
        debug_assert!(deep.end <= shallow.start || deep.start >= shallow.end);
        for i in deep {
            match self.child(i, /*meaningless*/ true) {
                Child::PageTable(pt) => {
                    let guard = pt.clone_shallow().lock();
                    let new_child = guard.make_copy(0..nr_subpage_per_huge::<C>(), 0..0);
                    new_frame.set_child_pt(i, new_child.into_raw(), /*meaningless*/ true);
                }
                Child::Frame(frame) => {
                    let prop = self.read_pte_prop(i);
                    new_frame.set_child_frame(i, frame.clone(), prop);
                }
                Child::None => {}
                Child::Untracked(_) => {
                    unreachable!();
                }
            }
        }
        for i in shallow {
            debug_assert_eq!(self.level(), C::NR_LEVELS);
            match self.child(i, /*meaningless*/ true) {
                Child::PageTable(pt) => {
                    new_frame.set_child_pt(i, pt.clone_shallow(), /*meaningless*/ true);
                }
                Child::None => {}
                Child::Frame(_) | Child::Untracked(_) => {
                    unreachable!();
                }
            }
        }
        new_frame
    }

    /// Remove a child if the child at the given index is present.
    pub(super) fn unset_child(&mut self, idx: usize, in_untracked_range: bool) {
        debug_assert!(idx < nr_subpage_per_huge::<C>());
        self.overwrite_pte(idx, None, in_untracked_range);
    }

    /// Set a child page table at a given index.
    pub(super) fn set_child_pt(
        &mut self,
        idx: usize,
        pt: RawPageTableNode<E, C>,
        in_untracked_range: bool,
    ) {
        // They should be ensured by the cursor.
        debug_assert!(idx < nr_subpage_per_huge::<C>());
        debug_assert_eq!(pt.level, self.level() - 1);
        let pte = Some(E::new_pt(pt.paddr()));
        self.overwrite_pte(idx, pte, in_untracked_range);
        // The ownership is transferred to a raw PTE. Don't drop the handle.
        let _ = ManuallyDrop::new(pt);
    }

    /// Map a frame at a given index.
    pub(super) fn set_child_frame(&mut self, idx: usize, frame: Frame, prop: PageProperty) {
        // They should be ensured by the cursor.
        debug_assert!(idx < nr_subpage_per_huge::<C>());
        debug_assert_eq!(frame.level(), self.level());
        let pte = Some(E::new_frame(frame.start_paddr(), self.level(), prop));
        self.overwrite_pte(idx, pte, false);
        // The ownership is transferred to a raw PTE. Don't drop the handle.
        let _ = ManuallyDrop::new(frame);
    }

    /// Set an untracked child frame at a given index.
    ///
    /// # Safety
    ///
    /// The caller must ensure that the physical address is valid and safe to map.
    pub(super) unsafe fn set_child_untracked(&mut self, idx: usize, pa: Paddr, prop: PageProperty) {
        // It should be ensured by the cursor.
        debug_assert!(idx < nr_subpage_per_huge::<C>());
        let pte = Some(E::new_frame(pa, self.level(), prop));
        self.overwrite_pte(idx, pte, true);
    }

    /// The number of mapped frames or page tables.
    /// This is to track if we can free itself.
    pub(super) fn nr_valid_children(&self) -> u16 {
        self.page.meta().nr_children
    }

    /// Read the info from a page table entry at a given index.
    pub(super) fn read_pte_prop(&self, idx: usize) -> PageProperty {
        self.read_pte(idx).prop()
    }

    /// Split the untracked huge page mapped at `idx` to smaller pages.
    pub(super) fn split_untracked_huge(&mut self, idx: usize) {
        // These should be ensured by the cursor.
        debug_assert!(idx < nr_subpage_per_huge::<C>());
        debug_assert!(self.level() > 1);

        let Child::Untracked(pa) = self.child(idx, false) else {
            panic!("`split_untracked_huge` not called on an untracked huge page");
        };
        let prop = self.read_pte_prop(idx);
        let mut new_frame = PageTableNode::<E, C>::alloc(self.level() - 1);
        for i in 0..nr_subpage_per_huge::<C>() {
            let small_pa = pa + i * page_size::<C>(self.level() - 1);
            // SAFETY: the index is within the bound and either physical address and
            // the property are valid.
            unsafe { new_frame.set_child_untracked(i, small_pa, prop) };
        }
        self.set_child_pt(idx, new_frame.into_raw(), true);
    }

    /// Protect an already mapped child at a given index.
    pub(super) fn protect(&mut self, idx: usize, prop: PageProperty) {
        let mut pte = self.read_pte(idx);
        debug_assert!(pte.is_present()); // This should be ensured by the cursor.
        pte.set_prop(prop);
        // SAFETY: the index is within the bound and the PTE is valid.
        unsafe {
            (self.as_ptr() as *mut E).add(idx).write(pte);
        }
    }

    pub(super) fn read_pte(&self, idx: usize) -> E {
        // It should be ensured by the cursor.
        debug_assert!(idx < nr_subpage_per_huge::<C>());
        // SAFETY: the index is within the bound and PTE is plain-old-data.
        unsafe { self.as_ptr().add(idx).read() }
    }

    fn start_paddr(&self) -> Paddr {
        self.page.paddr()
    }

    /// Replace a page table entry at a given index.
    ///
    /// This method will ensure that the child presented by the overwritten
    /// PTE is dropped, and the child count is updated.
    ///
    /// The caller in this module will ensure that the PTE points to initialized
    /// memory if the child is a page table.
    fn overwrite_pte(&mut self, idx: usize, pte: Option<E>, in_untracked_range: bool) {
        let existing_pte = self.read_pte(idx);
        if existing_pte.is_present() {
            // SAFETY: The index is within the bound and the address is aligned.
            // The validity of the PTE is checked within this module.
            // The safetiness also holds in the following branch.
            unsafe {
                (self.as_ptr() as *mut E)
                    .add(idx)
                    .write(pte.unwrap_or(E::new_absent()))
            };

            // Drop the child. We must set the PTE before dropping the child.
            // Just restore the handle and drop the handle.

            let paddr = existing_pte.paddr();
            // SAFETY: Both the `from_raw` operations here are safe as the physical
            // address is valid and casted from a handle.
            unsafe {
                if !existing_pte.is_last(self.level()) {
                    // This is a page table.
                    drop(Page::<PageTablePageMeta<E, C>>::from_raw(paddr));
                } else if !in_untracked_range {
                    // This is a frame.
                    drop(Page::<FrameMeta>::from_raw(paddr));
                }
            }

            // Update the child count.
            if pte.is_none() {
                // SAFETY: Here we have an exclusive access to the page.
                unsafe { self.page.meta_mut().nr_children -= 1 };
            }
        } else if let Some(e) = pte {
            // SAFETY: This is safe as described in the above branch.
            unsafe { (self.as_ptr() as *mut E).add(idx).write(e) };
            // SAFETY: Here we have an exclusive access to the page.
            unsafe { self.page.meta_mut().nr_children += 1 };
        }
    }

    fn as_ptr(&self) -> *const E {
        paddr_to_vaddr(self.start_paddr()) as *const E
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
        let paddr = page.paddr();
        let level = page.meta().level;
        // Drop the children.
        for i in 0..nr_subpage_per_huge::<C>() {
            // SAFETY: The index is within the bound and PTE is plain-old-data. The
            // address is aligned as well. We also have an exclusive access ensured
            // by reference counting.
            let pte_ptr = unsafe { (paddr_to_vaddr(paddr) as *const E).add(i) };
            // SAFETY: The pointer is valid and the PTE is plain-old-data.
            let pte = unsafe { pte_ptr.read() };
            if pte.is_present() {
                // Just restore the handle and drop the handle.
                if !pte.is_last(level) {
                    // This is a page table.
                    // SAFETY: The physical address must be casted from a handle to a
                    // page table node.
                    drop(unsafe { Page::<Self>::from_raw(pte.paddr()) });
                } else {
                    // This is a frame. You cannot drop a page table node that maps to
                    // untracked frames. This must be verified.
                    // SAFETY: The physical address must be casted from a handle to a
                    // frame.
                    drop(unsafe { Page::<FrameMeta>::from_raw(pte.paddr()) });
                }
            }
        }
        // Recycle this page table node.
        FRAME_ALLOCATOR
            .get()
            .unwrap()
            .lock()
            .dealloc(paddr / PAGE_SIZE, 1);
    }
}
