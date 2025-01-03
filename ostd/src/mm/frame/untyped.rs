// SPDX-License-Identifier: MPL-2.0

//! Untyped physical memory management.
//!
//! As detailed in [`crate::mm::frame`], untyped memory can be accessed with
//! relaxed rules but we cannot create references to them. This module provides
//! the declaration of untyped frames and segments, and the implementation of
//! extra functionalities (such as [`VmIo`]) for them.

use super::{meta::AnyFrameMeta, Frame, Segment};
use crate::{
    mm::{
        io::{FallibleVmRead, FallibleVmWrite, VmIo, VmReader, VmWriter},
        paddr_to_vaddr, Infallible,
    },
    Error, Result,
};

/// The metadata of untyped frame.
///
/// If a structure `M` implements [`AnyUFrameMeta`], it can be used as the
/// metadata of a type of untyped frames [`Frame<M>`]. All frames of such type
/// will be accessible as untyped memory.
pub trait AnyUFrameMeta: AnyFrameMeta {}

/// A smart pointer to an untyped frame with any metadata.
///
/// The metadata of the frame is not known at compile time but the frame must
/// be an untyped one. An [`UFrame`] as a parameter accepts any type of
/// untyped frame metadata.
///
/// The usage of this frame will not be changed while this object is alive.
pub type UFrame = Frame<dyn AnyUFrameMeta>;

/// Makes a structure usable as untyped frame metadata.
///
/// Directly implementing [`AnyFrameMeta`] is not safe since the size and
/// alignment must be checked. This macro provides a safe way to implement both
/// [`AnyFrameMeta`] and [`AnyUFrameMeta`] with compile-time checks.
///
/// If this macro is used for built-in typed frame metadata, it won't compile.
#[macro_export]
macro_rules! impl_untyped_frame_meta_for {
    // Implement without specifying the drop behavior.
    ($t:ty) => {
        static_assertions::const_assert!(
            size_of::<$t>() <= $crate::mm::frame::meta::FRAME_METADATA_MAX_SIZE
        );
        static_assertions::const_assert!(
            align_of::<$t>() <= $crate::mm::frame::meta::FRAME_METADATA_MAX_ALIGN
        );
        // SAFETY: The size and alignment of the structure are checked.
        unsafe impl $crate::mm::frame::meta::AnyFrameMeta for $t {
            fn is_untyped(&self) -> bool {
                true
            }
        }
        impl $crate::mm::frame::untyped::AnyUFrameMeta for $t {}
    };
    // Implement with a customized drop function.
    ($t:ty, $body:expr) => {
        static_assertions::const_assert!(
            size_of::<$t>() <= $crate::mm::frame::meta::FRAME_METADATA_MAX_SIZE
        );
        static_assertions::const_assert!(
            align_of::<$t>() <= $crate::mm::frame::meta::FRAME_METADATA_MAX_ALIGN
        );
        // SAFETY: The size and alignment of the structure are checked.
        // Outside OSTD the user cannot implement a `on_drop` method for typed
        // frames. And untyped frames can be safely read.
        unsafe impl $crate::mm::frame::meta::AnyFrameMeta for $t {
            fn on_drop(&mut self, reader: &mut $crate::mm::VmReader<$crate::mm::Infallible>) {
                $body
            }

            fn is_untyped(&self) -> bool {
                true
            }
        }
        impl $crate::mm::frame::untyped::AnyUFrameMeta for $t {}
    };
}

// A special case of untyped metadata is the unit type.
impl_untyped_frame_meta_for!(());

/// A physical memory range that is untyped.
///
/// Untyped frames or segments can be safely read and written by the kernel or
/// the user.
pub trait UntypedMem {
    /// Borrows a reader that can read the untyped memory.
    fn reader(&self) -> VmReader<'_, Infallible>;
    /// Borrows a writer that can write the untyped memory.
    fn writer(&self) -> VmWriter<'_, Infallible>;
}

macro_rules! impl_untyped_for {
    ($t:ident) => {
        impl<UM: AnyUFrameMeta + ?Sized> UntypedMem for $t<UM> {
            fn reader(&self) -> VmReader<'_, Infallible> {
                let ptr = paddr_to_vaddr(self.start_paddr()) as *const u8;
                // SAFETY: Only untyped frames are allowed to be read.
                unsafe { VmReader::from_kernel_space(ptr, self.size()) }
            }

            fn writer(&self) -> VmWriter<'_, Infallible> {
                let ptr = paddr_to_vaddr(self.start_paddr()) as *mut u8;
                // SAFETY: Only untyped frames are allowed to be written.
                unsafe { VmWriter::from_kernel_space(ptr, self.size()) }
            }
        }

        impl<UM: AnyUFrameMeta + ?Sized> VmIo for $t<UM> {
            fn read(&self, offset: usize, writer: &mut VmWriter) -> Result<()> {
                let read_len = writer.avail().min(self.size().saturating_sub(offset));
                // Do bound check with potential integer overflow in mind
                let max_offset = offset.checked_add(read_len).ok_or(Error::Overflow)?;
                if max_offset > self.size() {
                    return Err(Error::InvalidArgs);
                }
                let len = self
                    .reader()
                    .skip(offset)
                    .read_fallible(writer)
                    .map_err(|(e, _)| e)?;
                debug_assert!(len == read_len);
                Ok(())
            }

            fn write(&self, offset: usize, reader: &mut VmReader) -> Result<()> {
                let write_len = reader.remain().min(self.size().saturating_sub(offset));
                // Do bound check with potential integer overflow in mind
                let max_offset = offset.checked_add(write_len).ok_or(Error::Overflow)?;
                if max_offset > self.size() {
                    return Err(Error::InvalidArgs);
                }
                let len = self
                    .writer()
                    .skip(offset)
                    .write_fallible(reader)
                    .map_err(|(e, _)| e)?;
                debug_assert!(len == write_len);
                Ok(())
            }
        }
    };
}

impl_untyped_for!(Frame);
impl_untyped_for!(Segment);

// Here are implementations for `xarray`.

use core::{marker::PhantomData, mem::ManuallyDrop, ops::Deref};

/// `FrameRef` is a struct that can work as `&'a Frame<m>`.
///
/// This is solely useful for [`crate::collections::xarray`].
pub struct FrameRef<'a, M: AnyUFrameMeta + ?Sized> {
    inner: ManuallyDrop<Frame<M>>,
    _marker: PhantomData<&'a Frame<M>>,
}

impl<M: AnyUFrameMeta + ?Sized> Deref for FrameRef<'_, M> {
    type Target = Frame<M>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

// SAFETY: `Frame` is essentially an `*const MetaSlot` that could be used as a `*const` pointer.
// The pointer is also aligned to 4.
unsafe impl<M: AnyUFrameMeta + ?Sized> xarray::ItemEntry for Frame<M> {
    type Ref<'a>
        = FrameRef<'a, M>
    where
        Self: 'a;

    fn into_raw(self) -> *const () {
        let ptr = self.ptr;
        let _ = ManuallyDrop::new(self);
        ptr as *const ()
    }

    unsafe fn from_raw(raw: *const ()) -> Self {
        Self {
            ptr: raw as *const _,
            _marker: PhantomData,
        }
    }

    unsafe fn raw_as_ref<'a>(raw: *const ()) -> Self::Ref<'a> {
        Self::Ref {
            inner: ManuallyDrop::new(Frame {
                ptr: raw as *const _,
                _marker: PhantomData,
            }),
            _marker: PhantomData,
        }
    }
}
