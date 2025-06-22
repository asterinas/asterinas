// SPDX-License-Identifier: MPL-2.0

use core::{marker::PhantomData, mem::ManuallyDrop, ops::Deref, ptr::NonNull};

use super::{
    meta::{AnyFrameMeta, MetaSlot},
    Frame,
};
use crate::{mm::Paddr, sync::non_null::NonNullPtr};

/// A struct that can work as `&'a Frame<M>`.
#[derive(Debug)]
pub struct FrameRef<'a, M: AnyFrameMeta + ?Sized> {
    inner: ManuallyDrop<Frame<M>>,
    _marker: PhantomData<&'a Frame<M>>,
}

impl<M: AnyFrameMeta + ?Sized> FrameRef<'_, M> {
    /// Borrows the [`Frame`] at the physical address as a [`FrameRef`].
    ///
    /// # Safety
    ///
    /// The caller must ensure that:
    ///  - the frame outlives the created reference, so that the reference can
    ///    be seen as borrowed from that frame.
    ///  - the type of the [`FrameRef`] (`M`) matches the borrowed frame.
    pub(in crate::mm) unsafe fn borrow_paddr(raw: Paddr) -> Self {
        Self {
            // SAFETY: The caller ensures the safety.
            inner: ManuallyDrop::new(unsafe { Frame::from_raw(raw) }),
            _marker: PhantomData,
        }
    }
}

impl<M: AnyFrameMeta + ?Sized> Deref for FrameRef<'_, M> {
    type Target = Frame<M>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

// SAFETY: `Frame` is essentially a `*const MetaSlot` that could be used as a non-null
// `*const` pointer.
unsafe impl<M: AnyFrameMeta + ?Sized> NonNullPtr for Frame<M> {
    type Target = PhantomData<Self>;

    type Ref<'a>
        = FrameRef<'a, M>
    where
        Self: 'a;

    const ALIGN_BITS: u32 = core::mem::align_of::<MetaSlot>().trailing_zeros();

    fn into_raw(self) -> NonNull<Self::Target> {
        let ptr = NonNull::new(self.ptr.cast_mut()).unwrap();
        let _ = ManuallyDrop::new(self);
        ptr.cast()
    }

    unsafe fn from_raw(raw: NonNull<Self::Target>) -> Self {
        Self {
            ptr: raw.as_ptr().cast_const().cast(),
            _marker: PhantomData,
        }
    }

    unsafe fn raw_as_ref<'a>(raw: NonNull<Self::Target>) -> Self::Ref<'a> {
        Self::Ref {
            inner: ManuallyDrop::new(Frame {
                ptr: raw.as_ptr().cast_const().cast(),
                _marker: PhantomData,
            }),
            _marker: PhantomData,
        }
    }

    fn ref_as_raw(ptr_ref: Self::Ref<'_>) -> core::ptr::NonNull<Self::Target> {
        NonNull::new(ptr_ref.inner.ptr.cast_mut()).unwrap().cast()
    }
}
