// SPDX-License-Identifier: MPL-2.0

//! This module defines page table frame abstractions and the handle.
//!
//! The page table frame is also frequently referred to as a page table in many architectural
//! documentations. We also call it the page table node if emphasizing the tree structure.
//!
//! This module leverages the frame metadata to manage the page table frames, which makes it
//! easier to provide the following guarantees:
//!
//! The page table frame is not freed when it is still in use by:
//!    - a parent page table frame,
//!    - or a handle to a page table frame,
//!    - or a processor.
//! This is implemented by using a reference counter in the frame metadata. If the above
//! conditions are not met, the page table frame is ensured to be freed upon dropping the last
//! reference.
//!
//! One can acquire exclusive access to a page table frame using merely the physical address of
//! the page table frame. This is implemented by a lock in the frame metadata. Here the
//! exclusiveness is only ensured for kernel code, and the processor's MMU is able to access the
//! page table frame while a lock is held. So the modification to the PTEs should be done after
//! the initialization of the entity that the PTE points to. This is taken care in this module.
//!

use core::{marker::PhantomData, mem::ManuallyDrop, ops::Range, panic, sync::atomic::Ordering};

use super::{nr_subpage_per_huge, page_size, PageTableEntryTrait};
use crate::{
    arch::mm::{PageTableEntry, PagingConsts},
    vm::{
        frame::allocator::FRAME_ALLOCATOR, paddr_to_vaddr, page_prop::PageProperty, FrameMetaRef,
        FrameType, Paddr, PagingConstsTrait, PagingLevel, VmFrame, PAGE_SIZE,
    },
};

/// The raw handle to a page table frame.
///
/// This handle is a referencer of a page table frame. Thus creating and dropping it will affect
/// the reference count of the page table frame. If dropped the raw handle as the last reference,
/// the page table frame and subsequent children will be freed.
///
/// Only the CPU or a PTE can access a page table frame using a raw handle. To access the page
/// table frame from the kernel code, use the handle [`PageTableFrame`].
#[derive(Debug)]
pub(super) struct RawPageTableFrame<E: PageTableEntryTrait, C: PagingConstsTrait>(
    Paddr,
    PagingLevel,
    PhantomData<(E, C)>,
)
where
    [(); C::NR_LEVELS as usize]:;

impl<E: PageTableEntryTrait, C: PagingConstsTrait> RawPageTableFrame<E, C>
where
    [(); C::NR_LEVELS as usize]:,
{
    pub(super) fn paddr(&self) -> Paddr {
        self.0
    }

    /// Convert a raw handle to an accessible handle by pertaining the lock.
    pub(super) fn lock(self) -> PageTableFrame<E, C> {
        let meta = unsafe { FrameMetaRef::from_raw(self.0, 1) };
        let level = self.1;
        // Acquire the lock.
        while meta
            .counter8_1
            .compare_exchange(0, 1, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            core::hint::spin_loop();
        }
        // Prevent dropping the handle.
        let _ = ManuallyDrop::new(self);
        PageTableFrame::<E, C> {
            meta,
            newly_created: false,
            level,
            _phantom: PhantomData,
        }
    }

    /// Create a copy of the handle.
    pub(super) fn copy_handle(&self) -> Self {
        let meta = unsafe { FrameMetaRef::from_raw(self.0, 1) };
        // Increment the reference count.
        meta.counter32_1.fetch_add(1, Ordering::Relaxed);
        Self(self.0, self.1, PhantomData)
    }

    pub(super) fn nr_valid_children(&self) -> u16 {
        let meta = unsafe { FrameMetaRef::from_raw(self.0, 1) };
        meta.counter16_1.load(Ordering::Relaxed)
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
            vm::CachePolicy,
        };

        debug_assert_eq!(self.1, PagingConsts::NR_LEVELS);

        let last_activated_paddr = current_page_table_paddr();

        activate_page_table(self.0, CachePolicy::Writeback);

        if last_activated_paddr == self.0 {
            return;
        }

        // Increment the reference count of the current page table.

        FrameMetaRef::from_raw(self.0, 1)
            .counter32_1
            .fetch_add(1, Ordering::Relaxed);

        // Decrement the reference count of the last activated page table.

        // Boot page tables are not tracked with [`PageTableFrame`], but
        // all page tables after the boot stage are tracked.
        //
        // TODO: the `cpu_local` implementation currently is underpowered,
        // there's no need using `AtomicBool` here.
        crate::cpu_local! {
            static CURRENT_IS_BOOT_PT: AtomicBool = AtomicBool::new(true);
        }
        if !CURRENT_IS_BOOT_PT.load(Ordering::Acquire) {
            // Restore and drop the last activated page table.
            let _last_activated_pt =
                Self(last_activated_paddr, PagingConsts::NR_LEVELS, PhantomData);
        } else {
            CURRENT_IS_BOOT_PT.store(false, Ordering::Release);
        }
    }
}

impl<E: PageTableEntryTrait, C: PagingConstsTrait> Drop for RawPageTableFrame<E, C>
where
    [(); C::NR_LEVELS as usize]:,
{
    fn drop(&mut self) {
        let mut meta = unsafe { FrameMetaRef::from_raw(self.0, 1) };
        if meta.counter32_1.fetch_sub(1, Ordering::Release) == 1 {
            // A fence is needed here with the same reasons stated in the implementation of
            // `Arc::drop`: <https://doc.rust-lang.org/std/sync/struct.Arc.html#method.drop>.
            core::sync::atomic::fence(Ordering::Acquire);
            // Drop the children.
            for i in 0..nr_subpage_per_huge::<C>() {
                // SAFETY: the index is within the bound and PTE is plain-old-data. The
                // address is aligned as well. We also have an exclusive access ensured
                // by reference counting.
                let pte_ptr = unsafe { (paddr_to_vaddr(self.paddr()) as *const E).add(i) };
                let pte = unsafe { pte_ptr.read() };
                if pte.is_present() {
                    // Just restore the handle and drop the handle.
                    if !pte.is_last(self.1) {
                        // This is a page table.
                        let _dropping_raw = Self(pte.paddr(), self.1 - 1, PhantomData);
                    } else {
                        // This is a frame. You cannot drop a page table node that maps to
                        // untracked frames. This must be verified.
                        let frame_meta = unsafe { FrameMetaRef::from_raw(pte.paddr(), self.1) };
                        let _dropping_frame = VmFrame { meta: frame_meta };
                    }
                }
            }
            // SAFETY: the frame is initialized and the physical address points to initialized memory.
            // We also have and exclusive access ensured by reference counting.
            unsafe {
                meta.deref_mut().frame_type = FrameType::Free;
            }
            // Recycle this page table frame.
            FRAME_ALLOCATOR
                .get()
                .unwrap()
                .lock()
                .dealloc(self.0 / PAGE_SIZE, 1);
        }
    }
}

/// A mutable handle to a page table frame.
///
/// The page table frame can own a set of handles to children, ensuring that the children
/// don't outlive the page table frame. Cloning a page table frame will create a deep copy
/// of the page table. Dropping the page table frame will also drop all handles if the page
/// table frame has no references. You can set the page table frame as a child of another
/// page table frame.
#[derive(Debug)]
pub(super) struct PageTableFrame<
    E: PageTableEntryTrait = PageTableEntry,
    C: PagingConstsTrait = PagingConsts,
> where
    [(); C::NR_LEVELS as usize]:,
{
    pub(super) meta: FrameMetaRef,
    /// This is an optimization to save a few atomic operations on the lock.
    ///
    /// If the handle is newly created using [`Self::alloc`], this is true and there's no need
    /// to acquire the lock since the handle is exclusive. However if the handle is acquired
    /// from a [`RawPageTableFrame`], this is false and the lock should be acquired.
    newly_created: bool,
    /// The level of the page table frame. This is needed because we cannot tell from a PTE
    /// alone if it is a page table or a frame.
    level: PagingLevel,
    _phantom: core::marker::PhantomData<(E, C)>,
}

/// A child of a page table frame.
#[derive(Debug)]
pub(super) enum Child<E: PageTableEntryTrait = PageTableEntry, C: PagingConstsTrait = PagingConsts>
where
    [(); C::NR_LEVELS as usize]:,
{
    PageTable(RawPageTableFrame<E, C>),
    Frame(VmFrame),
    /// Frames not tracked by handles.
    Untracked(Paddr),
    None,
}

impl<E: PageTableEntryTrait, C: PagingConstsTrait> PageTableFrame<E, C>
where
    [(); C::NR_LEVELS as usize]:,
{
    /// Allocate a new empty page table frame.
    ///
    /// This function returns an owning handle. The newly created handle does not
    /// set the lock bit for performance as it is exclusive and unlocking is an
    /// extra unnecessary expensive operation.
    pub(super) fn alloc(level: PagingLevel) -> Self {
        let frame = FRAME_ALLOCATOR.get().unwrap().lock().alloc(1).unwrap() * PAGE_SIZE;
        let mut meta = unsafe { FrameMetaRef::from_raw(frame, 1) };
        // The reference count is initialized to 1.
        meta.counter32_1.store(1, Ordering::Relaxed);
        // The lock is initialized to 0.
        meta.counter8_1.store(0, Ordering::Release);
        // SAFETY: here we have an exlusive access since it's just initialized.
        unsafe {
            meta.deref_mut().frame_type = FrameType::PageTable;
        }

        // Zero out the page table frame.
        let ptr = paddr_to_vaddr(meta.paddr()) as *mut u8;
        unsafe { core::ptr::write_bytes(ptr, 0, PAGE_SIZE) };

        Self {
            meta,
            newly_created: true,
            level,
            _phantom: PhantomData,
        }
    }

    /// Convert the handle into a raw handle to be stored in a PTE or CPU.
    pub(super) fn into_raw(mut self) -> RawPageTableFrame<E, C> {
        if !self.newly_created {
            self.meta.counter8_1.store(0, Ordering::Release);
        } else {
            self.newly_created = false;
        }
        let raw = RawPageTableFrame(self.start_paddr(), self.level, PhantomData);
        let _ = ManuallyDrop::new(self);
        raw
    }

    /// Get a raw handle while still preserving the original handle.
    pub(super) fn clone_raw(&self) -> RawPageTableFrame<E, C> {
        self.meta.counter32_1.fetch_add(1, Ordering::Relaxed);
        RawPageTableFrame(self.start_paddr(), self.level, PhantomData)
    }

    /// Get an extra reference of the child at the given index.
    pub(super) fn child(&self, idx: usize, tracked: bool) -> Child<E, C> {
        debug_assert!(idx < nr_subpage_per_huge::<C>());
        let pte = self.read_pte(idx);
        if !pte.is_present() {
            Child::None
        } else {
            let paddr = pte.paddr();
            if !pte.is_last(self.level) {
                let meta = unsafe { FrameMetaRef::from_raw(paddr, 1) };
                // This is the handle count. We are creating a new handle thus increment the counter.
                meta.counter32_1.fetch_add(1, Ordering::Relaxed);
                Child::PageTable(RawPageTableFrame(paddr, self.level - 1, PhantomData))
            } else if tracked {
                let meta = unsafe { FrameMetaRef::from_raw(paddr, self.level) };
                // This is the handle count. We are creating a new handle thus increment the counter.
                meta.counter32_1.fetch_add(1, Ordering::Relaxed);
                Child::Frame(VmFrame { meta })
            } else {
                Child::Untracked(paddr)
            }
        }
    }

    /// Make a copy of the page table frame.
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
        let mut new_frame = Self::alloc(self.level);
        debug_assert!(deep.end <= nr_subpage_per_huge::<C>());
        debug_assert!(shallow.end <= nr_subpage_per_huge::<C>());
        debug_assert!(deep.end <= shallow.start || deep.start >= shallow.end);
        for i in deep {
            match self.child(i, /*meaningless*/ true) {
                Child::PageTable(pt) => {
                    let guard = pt.copy_handle().lock();
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
            debug_assert_eq!(self.level, C::NR_LEVELS);
            match self.child(i, /*meaningless*/ true) {
                Child::PageTable(pt) => {
                    new_frame.set_child_pt(i, pt.copy_handle(), /*meaningless*/ true);
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
    pub(super) fn unset_child(&self, idx: usize, in_untracked_range: bool) {
        debug_assert!(idx < nr_subpage_per_huge::<C>());
        self.overwrite_pte(idx, None, in_untracked_range);
    }

    /// Set a child page table at a given index.
    pub(super) fn set_child_pt(
        &mut self,
        idx: usize,
        pt: RawPageTableFrame<E, C>,
        in_untracked_range: bool,
    ) {
        // They should be ensured by the cursor.
        debug_assert!(idx < nr_subpage_per_huge::<C>());
        debug_assert_eq!(pt.1, self.level - 1);
        let pte = Some(E::new_pt(pt.paddr()));
        self.overwrite_pte(idx, pte, in_untracked_range);
        // The ownership is transferred to a raw PTE. Don't drop the handle.
        let _ = ManuallyDrop::new(pt);
    }

    /// Map a frame at a given index.
    pub(super) fn set_child_frame(&mut self, idx: usize, frame: VmFrame, prop: PageProperty) {
        // They should be ensured by the cursor.
        debug_assert!(idx < nr_subpage_per_huge::<C>());
        debug_assert_eq!(frame.level(), self.level);
        let pte = Some(E::new_frame(frame.start_paddr(), self.level, prop));
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
        let pte = Some(E::new_frame(pa, self.level, prop));
        self.overwrite_pte(idx, pte, true);
    }

    /// The number of mapped frames or page tables.
    /// This is to track if we can free itself.
    pub(super) fn nr_valid_children(&self) -> u16 {
        self.meta.counter16_1.load(Ordering::Relaxed)
    }

    /// Read the info from a page table entry at a given index.
    pub(super) fn read_pte_prop(&self, idx: usize) -> PageProperty {
        self.read_pte(idx).prop()
    }

    /// Split the untracked huge page mapped at `idx` to smaller pages.
    pub(super) fn split_untracked_huge(&mut self, idx: usize) {
        // These should be ensured by the cursor.
        debug_assert!(idx < nr_subpage_per_huge::<C>());
        debug_assert!(self.level > 1);

        let Child::Untracked(pa) = self.child(idx, false) else {
            panic!("`split_untracked_huge` not called on an untracked huge page");
        };
        let prop = self.read_pte_prop(idx);
        let mut new_frame = PageTableFrame::<E, C>::alloc(self.level - 1);
        for i in 0..nr_subpage_per_huge::<C>() {
            let small_pa = pa + i * page_size::<C>(self.level - 1);
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
        self.meta.paddr()
    }

    /// Replace a page table entry at a given index.
    ///
    /// This method will ensure that the child presented by the overwritten
    /// PTE is dropped, and the child count is updated.
    ///
    /// The caller in this module will ensure that the PTE points to initialized
    /// memory if the child is a page table.
    fn overwrite_pte(&self, idx: usize, pte: Option<E>, in_untracked_range: bool) {
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

            // Drop the child. We must set the PTE before dropping the child. To
            // drop the child just restore the handle and drop the handle.

            let paddr = existing_pte.paddr();
            if !existing_pte.is_last(self.level) {
                // This is a page table.
                let _dropping_raw = RawPageTableFrame::<E, C>(paddr, self.level - 1, PhantomData);
            } else if !in_untracked_range {
                // This is a frame.
                let meta = unsafe { FrameMetaRef::from_raw(paddr, self.level) };
                let _dropping_frame = VmFrame { meta };
            }

            if pte.is_none() {
                // Decrement the child count.
                self.meta.counter16_1.fetch_sub(1, Ordering::Relaxed);
            }
        } else if let Some(e) = pte {
            unsafe { (self.as_ptr() as *mut E).add(idx).write(e) };

            // Increment the child count.
            self.meta.counter16_1.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn as_ptr(&self) -> *const E {
        paddr_to_vaddr(self.start_paddr()) as *const E
    }
}

impl<E: PageTableEntryTrait, C: PagingConstsTrait> Drop for PageTableFrame<E, C>
where
    [(); C::NR_LEVELS as usize]:,
{
    fn drop(&mut self) {
        // Release the lock.
        if !self.newly_created {
            self.meta.counter8_1.store(0, Ordering::Release);
        }
        // Drop the frame by `RawPageTableFrame::drop`.
        let _dropping_raw = RawPageTableFrame::<E, C>(self.start_paddr(), self.level, PhantomData);
    }
}
