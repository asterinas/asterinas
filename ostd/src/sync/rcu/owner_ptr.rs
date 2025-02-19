// SPDX-License-Identifier: MPL-2.0

use core::ptr::NonNull;

use crate::prelude::*;

/// A trait that abstracts pointers that have the ownership of the objects they
/// refer to.
///
/// The most typical examples smart pointer types like `Box<T>` and `Arc<T>`,
/// which can be converted to and from the raw pointer type of `*const T`.
///
/// # Safety
///
/// This trait must be implemented correctly (according to the doc comments for
/// each method). Types like [`Rcu`] rely on this assumption to safely use the
/// raw pointers.
///
/// [`Rcu`]: super::Rcu
pub unsafe trait OwnerPtr: Send + 'static {
    /// The target type that this pointer refers to.
    // TODO: allow ?Sized
    type Target;

    /// Creates a new pointer with the given value.
    fn new(value: Self::Target) -> Self;

    /// Converts to a raw pointer.
    ///
    /// Each call to `into_raw` must be paired with a call to `from_raw`
    /// in order to avoid memory leakage.
    ///
    /// The resulting raw pointer must be valid to be immutably accessed
    /// or borrowed until `from_raw` is called.
    fn into_raw(self) -> NonNull<Self::Target>;

    /// Converts back from a raw pointer.
    ///
    /// # Safety
    ///
    /// 1. The raw pointer must have been previously returned by a call to
    ///    `into_raw`.
    /// 2. The raw pointer must not be used after calling `from_raw`.
    ///
    /// Note that the second point is a hard requirement: Even if the
    /// resulting value has not (yet) been dropped, the pointer cannot be
    /// used because it may break Rust aliasing rules (e.g., `Box<T>`
    /// requires the pointer to be unique and thus _never_ aliased).
    unsafe fn from_raw(ptr: NonNull<Self::Target>) -> Self;
}

unsafe impl<T: Send + 'static> OwnerPtr for Box<T> {
    type Target = T;

    fn new(value: Self::Target) -> Self {
        Box::new(value)
    }

    fn into_raw(self) -> NonNull<Self::Target> {
        let ptr = Box::into_raw(self);

        // SAFETY: The pointer representing a `Box` can never be NULL.
        unsafe { NonNull::new_unchecked(ptr) }
    }

    unsafe fn from_raw(ptr: NonNull<Self::Target>) -> Self {
        let ptr = ptr.as_ptr();

        // SAFETY: The safety is upheld by the caller.
        unsafe { Box::from_raw(ptr) }
    }
}

unsafe impl<T: Send + Sync + 'static> OwnerPtr for Arc<T> {
    type Target = T;

    fn new(value: Self::Target) -> Self {
        Arc::new(value)
    }

    fn into_raw(self) -> NonNull<Self::Target> {
        let ptr = Arc::into_raw(self).cast_mut();

        // SAFETY: The pointer representing an `Arc` can never be NULL.
        unsafe { NonNull::new_unchecked(ptr) }
    }

    unsafe fn from_raw(ptr: NonNull<Self::Target>) -> Self {
        let ptr = ptr.as_ptr().cast_const();

        // SAFETY: The safety is upheld by the caller.
        unsafe { Arc::from_raw(ptr) }
    }
}
