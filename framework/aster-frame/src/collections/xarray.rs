// SPDX-License-Identifier: MPL-2.0

//! This module introduces the xarray crate and provides relevant support and interfaces for `XArray`.
extern crate xarray as xarray_crate;

use core::{marker::PhantomData, mem::ManuallyDrop, ops::Deref};

use xarray_crate::ItemEntry;
pub use xarray_crate::{Cursor, CursorMut, XArray, XMark};

use crate::vm::{FrameMetaRef, VmFrame};

/// `VmFrameRef` is a struct that can work as `&'a VmFrame`.
pub struct VmFrameRef<'a> {
    inner: ManuallyDrop<VmFrame>,
    _marker: PhantomData<&'a VmFrame>,
}

impl<'a> Deref for VmFrameRef<'a> {
    type Target = VmFrame;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

// SAFETY: `VmFrame` is essentially an `*const FrameMeta` that could be used as a `*const` pointer.
// The pointer is also aligned to 4.
unsafe impl ItemEntry for VmFrame {
    type Ref<'a> = VmFrameRef<'a> where Self: 'a;

    fn into_raw(self) -> *const () {
        let ptr = self.meta.inner;
        let _ = ManuallyDrop::new(self);
        ptr.cast()
    }

    unsafe fn from_raw(raw: *const ()) -> Self {
        Self {
            meta: FrameMetaRef { inner: raw.cast() },
        }
    }

    unsafe fn raw_as_ref<'a>(raw: *const ()) -> Self::Ref<'a> {
        VmFrameRef {
            inner: ManuallyDrop::new(VmFrame::from_raw(raw.cast())),
            _marker: PhantomData,
        }
    }
}
