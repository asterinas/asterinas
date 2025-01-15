// SPDX-License-Identifier: MPL-2.0

//! Frame (physical memory page) management.
//!
//! A frame is an aligned, contiguous range of bytes in physical memory. The
//! sizes of base frames and huge frames (that are mapped as "huge pages") are
//! architecture-dependent. A frame can be mapped to virtual address spaces
//! using the page table.
//!
//! Frames can be accessed through frame handles, namely, [`Frame`]. A frame
//! handle is a reference-counted pointer to a frame. When all handles to a
//! frame are dropped, the frame is released and can be reused.  Contiguous
//! frames are managed with [`Segment`].
//!
//! There are various kinds of frames. The top-level grouping of frame kinds
//! are "typed" frames and "untyped" frames. Typed frames host Rust objects
//! that must follow the visibility, lifetime and borrow rules of Rust, thus
//! not being able to be directly manipulated. Untyped frames are raw memory
//! that can be manipulated directly. So only untyped frames can be
//!  - safely shared to external entities such as device drivers or user-space
//!    applications.
//!  - or directly manipulated with readers and writers that neglect Rust's
//!    "alias XOR mutability" rule.
//!
//! The kind of a frame is determined by the type of its metadata. Untyped
//! frames have its metadata type that implements the [`UntypedFrameMeta`]
//! trait, while typed frames don't.
//!
//! Frames can have dedicated metadata, which is implemented in the [`meta`]
//! module. The reference count and usage of a frame are stored in the metadata
//! as well, leaving the handle only a pointer to the metadata slot. Users
//! can create custom metadata types by implementing the [`AnyFrameMeta`] trait.

pub mod allocator;
pub mod meta;
pub mod segment;
pub mod untyped;

#[cfg(ktest)]
mod test;

use core::{
    marker::PhantomData,
    sync::atomic::{AtomicU32, AtomicUsize, Ordering},
};

use meta::{
    mapping, AnyFrameMeta, MetaSlot, FRAME_METADATA_MAX_ALIGN, FRAME_METADATA_MAX_SIZE,
    REF_COUNT_UNUSED,
};
pub use segment::Segment;
use untyped::{AnyUFrameMeta, UFrame};

use super::{PagingLevel, PAGE_SIZE};
use crate::mm::{Paddr, PagingConsts, Vaddr};

static MAX_PADDR: AtomicUsize = AtomicUsize::new(0);

/// A smart pointer to a frame.
///
/// A frame is a contiguous range of bytes in physical memory. The [`Frame`]
/// type is a smart pointer to a frame that is reference-counted.
///
/// Frames are associated with metadata. The type of the metadata `M` is
/// determines the kind of the frame. If `M` implements [`AnyUFrameMeta`], the
/// frame is a untyped frame. Otherwise, it is a typed frame.
#[derive(Debug)]
#[repr(transparent)]
pub struct Frame<M: AnyFrameMeta + ?Sized> {
    // TODO: We may use a `NonNull<M>` here to make the frame a maybe-fat
    // pointer and implement `CoerceUnsized` to avoid `From`s. However this is
    // not quite feasible currently because we cannot cast a must-be-fat
    // pointer (`*const dyn AnyFrameMeta`) to a maybe-fat pointer (`NonNull<M>`).
    ptr: *const MetaSlot,
    _marker: PhantomData<M>,
}

unsafe impl<M: AnyFrameMeta + ?Sized> Send for Frame<M> {}

unsafe impl<M: AnyFrameMeta + ?Sized> Sync for Frame<M> {}

impl<M: AnyFrameMeta> Frame<M> {
    /// Gets a [`Frame`] with a specific usage from a raw, unused page.
    ///
    /// The caller should provide the initial metadata of the page.
    ///
    /// # Panics
    ///
    /// The function panics if:
    ///  - the physical address is out of bound or not aligned;
    ///  - the page is already in use.
    pub fn from_unused(paddr: Paddr, metadata: M) -> Self {
        assert!(paddr % PAGE_SIZE == 0);
        assert!(paddr < MAX_PADDR.load(Ordering::Relaxed) as Paddr);

        // Checking unsafe preconditions of the `AnyFrameMeta` trait.
        debug_assert!(size_of::<M>() <= FRAME_METADATA_MAX_SIZE);
        debug_assert!(align_of::<M>() <= FRAME_METADATA_MAX_ALIGN);

        let vaddr = mapping::frame_to_meta::<PagingConsts>(paddr);
        let ptr = vaddr as *const MetaSlot;

        // SAFETY: `ptr` points to a valid `MetaSlot` that will never be mutably borrowed, so taking an
        // immutable reference to it is always safe.
        let slot = unsafe { &*ptr };

        // `Acquire` pairs with the `Release` in `drop_last_in_place` and ensures the metadata
        // initialization won't be reordered before this memory compare-and-exchange.
        slot.ref_count
            .compare_exchange(REF_COUNT_UNUSED, 0, Ordering::Acquire, Ordering::Relaxed)
            .expect("Frame already in use when trying to get a new handle");

        // SAFETY: We have exclusive access to the page metadata. These fields are mutably
        // borrowed only once.
        let vtable_ptr = unsafe { &mut *slot.vtable_ptr.get() };
        vtable_ptr.write(core::ptr::metadata(&metadata as &dyn AnyFrameMeta));

        // SAFETY:
        // 1. `ptr` points to the first field of `MetaSlot` (guaranteed by `repr(C)`), which is the
        //    metadata storage.
        // 2. The size and the alignment of the metadata storage is large enough to hold `M`
        //    (guaranteed by the safety requirement of the `AnyFrameMeta` trait).
        // 3. We have exclusive access to the metadata storage (guaranteed by the reference count).
        unsafe { ptr.cast::<M>().cast_mut().write(metadata) };

        // Assuming no one can create a `Frame` instance directly from the page address, `Relaxed`
        // is fine here. Otherwise, we should use `Release` to ensure that the metadata
        // initialization won't be reordered after this memory store.
        slot.ref_count.store(1, Ordering::Relaxed);

        Self {
            ptr,
            _marker: PhantomData,
        }
    }

    /// Gets the metadata of this page.
    pub fn meta(&self) -> &M {
        // SAFETY: `self.ptr` points to the metadata storage which is valid to
        // be immutably borrowed as `M` because the type is correct, it lives
        // under the given lifetime, and no one will mutably borrow the page
        // metadata after initialization.
        unsafe { &*self.ptr.cast() }
    }
}

impl<M: AnyFrameMeta + ?Sized> Frame<M> {
    /// Gets the physical address of the start of the frame.
    pub fn start_paddr(&self) -> Paddr {
        mapping::meta_to_frame::<PagingConsts>(self.ptr as Vaddr)
    }

    /// Gets the paging level of this page.
    ///
    /// This is the level of the page table entry that maps the frame,
    /// which determines the size of the frame.
    ///
    /// Currently, the level is always 1, which means the frame is a regular
    /// page frame.
    pub const fn level(&self) -> PagingLevel {
        1
    }

    /// Gets the size of this page in bytes.
    pub const fn size(&self) -> usize {
        PAGE_SIZE
    }

    /// Gets the dyncamically-typed metadata of this frame.
    ///
    /// If the type is known at compile time, use [`Frame::meta`] instead.
    pub fn dyn_meta(&self) -> &dyn AnyFrameMeta {
        let slot = self.slot();

        // SAFETY: The page metadata is valid to be borrowed immutably, since it will never be
        // borrowed mutably after initialization.
        let vtable_ptr = unsafe { &*slot.vtable_ptr.get() };

        // SAFETY: The page metadata is initialized and valid.
        let vtable_ptr = *unsafe { vtable_ptr.assume_init_ref() };

        let meta_ptr: *const dyn AnyFrameMeta = core::ptr::from_raw_parts(self.ptr, vtable_ptr);

        // SAFETY: `self.ptr` points to the metadata storage which is valid to be immutably
        // borrowed under `vtable_ptr` because the vtable is correct, it lives under the given
        // lifetime, and no one will mutably borrow the page metadata after initialization.
        unsafe { &*meta_ptr }
    }

    /// Gets the reference count of the frame.
    ///
    /// It returns the number of all references to the frame, including all the
    /// existing frame handles ([`Frame`], [`Frame<dyn AnyFrameMeta>`]), and all
    /// the mappings in the page table that points to the frame.
    ///
    /// # Safety
    ///
    /// The function is safe to call, but using it requires extra care. The
    /// reference count can be changed by other threads at any time including
    /// potentially between calling this method and acting on the result.
    pub fn reference_count(&self) -> u32 {
        self.ref_count().load(Ordering::Relaxed)
    }

    fn ref_count(&self) -> &AtomicU32 {
        unsafe { &(*self.ptr).ref_count }
    }

    /// Forgets the handle to the frame.
    ///
    /// This will result in the frame being leaked without calling the custom dropper.
    ///
    /// A physical address to the frame is returned in case the frame needs to be
    /// restored using [`Frame::from_raw`] later. This is useful when some architectural
    /// data structures need to hold the frame handle such as the page table.
    #[allow(unused)]
    pub(in crate::mm) fn into_raw(self) -> Paddr {
        let paddr = self.start_paddr();
        core::mem::forget(self);
        paddr
    }

    /// Restores a forgotten `Frame` from a physical address.
    ///
    /// # Safety
    ///
    /// The caller should only restore a `Frame` that was previously forgotten using
    /// [`Frame::into_raw`].
    ///
    /// And the restoring operation should only be done once for a forgotten
    /// `Frame`. Otherwise double-free will happen.
    ///
    /// Also, the caller ensures that the usage of the frame is correct. There's
    /// no checking of the usage in this function.
    pub(in crate::mm) unsafe fn from_raw(paddr: Paddr) -> Self {
        let vaddr = mapping::frame_to_meta::<PagingConsts>(paddr);
        let ptr = vaddr as *const MetaSlot;

        Self {
            ptr,
            _marker: PhantomData,
        }
    }

    fn slot(&self) -> &MetaSlot {
        // SAFETY: `ptr` points to a valid `MetaSlot` that will never be
        // mutably borrowed, so taking an immutable reference to it is safe.
        unsafe { &*self.ptr }
    }
}

impl<M: AnyFrameMeta + ?Sized> Clone for Frame<M> {
    fn clone(&self) -> Self {
        // SAFETY: We have already held a reference to the frame.
        unsafe { self.slot().inc_ref_count() };

        Self {
            ptr: self.ptr,
            _marker: PhantomData,
        }
    }
}

impl<M: AnyFrameMeta + ?Sized> Drop for Frame<M> {
    fn drop(&mut self) {
        let last_ref_cnt = self.slot().ref_count.fetch_sub(1, Ordering::Release);
        debug_assert!(last_ref_cnt != 0 && last_ref_cnt != REF_COUNT_UNUSED);

        if last_ref_cnt == 1 {
            // A fence is needed here with the same reasons stated in the implementation of
            // `Arc::drop`: <https://doc.rust-lang.org/std/sync/struct.Arc.html#method.drop>.
            core::sync::atomic::fence(Ordering::Acquire);

            // SAFETY: this is the last reference and is about to be dropped.
            unsafe {
                meta::drop_last_in_place(self.ptr as *mut MetaSlot);
            }
        }
    }
}

impl<M: AnyFrameMeta> TryFrom<Frame<dyn AnyFrameMeta>> for Frame<M> {
    type Error = Frame<dyn AnyFrameMeta>;

    /// Tries converting a [`Frame<dyn AnyFrameMeta>`] into the statically-typed [`Frame`].
    ///
    /// If the usage of the frame is not the same as the expected usage, it will
    /// return the dynamic frame itself as is.
    fn try_from(dyn_frame: Frame<dyn AnyFrameMeta>) -> Result<Self, Self::Error> {
        if (dyn_frame.dyn_meta() as &dyn core::any::Any).is::<M>() {
            // SAFETY: The metadata is coerceable and the struct is transmutable.
            Ok(unsafe { core::mem::transmute::<Frame<dyn AnyFrameMeta>, Frame<M>>(dyn_frame) })
        } else {
            Err(dyn_frame)
        }
    }
}

impl<M: AnyFrameMeta> From<Frame<M>> for Frame<dyn AnyFrameMeta> {
    fn from(frame: Frame<M>) -> Self {
        // SAFETY: The metadata is coerceable and the struct is transmutable.
        unsafe { core::mem::transmute(frame) }
    }
}

impl<M: AnyUFrameMeta> From<Frame<M>> for UFrame {
    fn from(frame: Frame<M>) -> Self {
        // SAFETY: The metadata is coerceable and the struct is transmutable.
        unsafe { core::mem::transmute(frame) }
    }
}

impl From<UFrame> for Frame<dyn AnyFrameMeta> {
    fn from(frame: UFrame) -> Self {
        // SAFETY: The metadata is coerceable and the struct is transmutable.
        unsafe { core::mem::transmute(frame) }
    }
}

impl TryFrom<Frame<dyn AnyFrameMeta>> for UFrame {
    type Error = Frame<dyn AnyFrameMeta>;

    /// Tries converting a [`Frame<dyn AnyFrameMeta>`] into [`UFrame`].
    ///
    /// If the usage of the frame is not the same as the expected usage, it will
    /// return the dynamic frame itself as is.
    fn try_from(dyn_frame: Frame<dyn AnyFrameMeta>) -> Result<Self, Self::Error> {
        if dyn_frame.dyn_meta().is_untyped() {
            // SAFETY: The metadata is coerceable and the struct is transmutable.
            Ok(unsafe { core::mem::transmute::<Frame<dyn AnyFrameMeta>, UFrame>(dyn_frame) })
        } else {
            Err(dyn_frame)
        }
    }
}

/// Increases the reference count of the frame by one.
///
/// # Safety
///
/// The caller should ensure the following conditions:
///  1. The physical address must represent a valid frame;
///  2. The caller must have already held a reference to the frame.
pub(in crate::mm) unsafe fn inc_frame_ref_count(paddr: Paddr) {
    debug_assert!(paddr % PAGE_SIZE == 0);
    debug_assert!(paddr < MAX_PADDR.load(Ordering::Relaxed) as Paddr);

    let vaddr: Vaddr = mapping::frame_to_meta::<PagingConsts>(paddr);
    // SAFETY: `vaddr` points to a valid `MetaSlot` that will never be mutably borrowed, so taking
    // an immutable reference to it is always safe.
    let slot = unsafe { &*(vaddr as *const MetaSlot) };

    // SAFETY: We have already held a reference to the frame.
    unsafe { slot.inc_ref_count() };
}
