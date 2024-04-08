// SPDX-License-Identifier: MPL-2.0

//! This module introduces the xarray crate and provides relevant support and interfaces for `XArray`.
extern crate xarray as xarray_crate;

use alloc::sync::Arc;
use core::{marker::PhantomData, mem::ManuallyDrop, ops::Deref};

use xarray_crate::ItemEntry;
pub use xarray_crate::{Cursor, CursorMut, XArray, XMark};

use crate::vm::VmFrame;

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

// SAFETY: `VmFrame` is essentially an `Arc` smart pointer that points to a location which is aligned to 4,
// meeting the requirements of the `ItemEntry` for `XArray`.
unsafe impl ItemEntry for VmFrame {
    type Ref<'a> = VmFrameRef<'a> where Self: 'a;

    fn into_raw(self) -> *const () {
        let ptr = Arc::as_ptr(&self.frame_index);
        let _ = ManuallyDrop::new(self);
        ptr.cast()
    }

    unsafe fn from_raw(raw: *const ()) -> Self {
        Self {
            frame_index: Arc::from_raw(raw.cast()),
        }
    }

    unsafe fn raw_as_ref<'a>(raw: *const ()) -> Self::Ref<'a> {
        VmFrameRef {
            inner: ManuallyDrop::new(VmFrame::from_raw(raw.cast())),
            _marker: PhantomData,
        }
    }
}
