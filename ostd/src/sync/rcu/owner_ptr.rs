// SPDX-License-Identifier: MPL-2.0

use core::ptr::NonNull;

use crate::prelude::*;

/// A trait that abstracts pointers that have the ownership of the objects they
/// refer to.
///
/// The most typical examples smart pointer types like `Box<T>` and `Arc<T>`.
///
/// which can be converted to and from the raw pointer type of `*const T`.
pub trait OwnerPtr {
    /// The target type that this pointer refers to.
    // TODO: allow ?Sized
    type Target;

    /// Converts to a raw pointer.
    ///
    /// If `Self` owns the object that it refers to (e.g., `Box<_>`), then
    /// each call to `into_raw` must be paired with a call to `from_raw`
    /// in order to avoid memory leakage.
    fn into_raw(self) -> *const Self::Target;

    /// Converts back from a raw pointer.
    ///
    /// # Safety
    ///
    /// The raw pointer must have been previously returned by a call to `into_raw`.
    unsafe fn from_raw(ptr: *const Self::Target) -> Self;
}

impl<T> OwnerPtr for Box<T> {
    type Target = T;

    fn into_raw(self) -> *const Self::Target {
        Box::into_raw(self) as *const _
    }

    unsafe fn from_raw(ptr: *const Self::Target) -> Self {
        Box::from_raw(ptr as *mut _)
    }
}

impl<T> OwnerPtr for Arc<T> {
    type Target = T;

    fn into_raw(self) -> *const Self::Target {
        Arc::into_raw(self)
    }

    unsafe fn from_raw(ptr: *const Self::Target) -> Self {
        Arc::from_raw(ptr)
    }
}

impl<P> OwnerPtr for Option<P>
where
    P: OwnerPtr,
    // We cannot support fat pointers, e.g., when `Target: dyn Trait`.
    // This is because Rust does not allow fat null pointers. Yet,
    // we need the null pointer to represent `None`.
    // See https://github.com/rust-lang/rust/issues/66316.
    <P as OwnerPtr>::Target: Sized,
{
    type Target = P::Target;

    fn into_raw(self) -> *const Self::Target {
        self.map(|p| <P as OwnerPtr>::into_raw(p))
            .unwrap_or(core::ptr::null())
    }

    unsafe fn from_raw(ptr: *const Self::Target) -> Self {
        if ptr.is_null() {
            Some(<P as OwnerPtr>::from_raw(ptr))
        } else {
            None
        }
    }
}
