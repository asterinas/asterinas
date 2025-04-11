// SPDX-License-Identifier: MPL-2.0

//! This module provides a trait and some auxiliary types to help abstract and
//! work with non-null pointers.

mod either;

use alloc::sync::Weak;
use core::{marker::PhantomData, mem::ManuallyDrop, ops::Deref, ptr::NonNull};

use crate::prelude::*;

/// A trait that abstracts non-null pointers.
///
/// All common smart pointer types such as `Box<T>`,  `Arc<T>`, and `Weak<T>`
/// implement this trait as they can be converted to and from the raw pointer
/// type of `*const T`.
///
/// # Safety
///
/// This trait must be implemented correctly (according to the doc comments for
/// each method). Types like [`Rcu`] rely on this assumption to safely use the
/// raw pointers.
///
/// [`Rcu`]: super::Rcu
pub unsafe trait NonNullPtr: 'static {
    /// The target type that this pointer refers to.
    // TODO: Support `Target: ?Sized`.
    type Target;

    /// A type that behaves just like a shared reference to the `NonNullPtr`.
    type Ref<'a>
    where
        Self: 'a;

    /// The power of two of the pointer alignment.
    const ALIGN_BITS: u32;

    /// Converts to a raw pointer.
    ///
    /// Each call to `into_raw` must be paired with a call to `from_raw`
    /// in order to avoid memory leakage.
    ///
    /// The lower [`Self::ALIGN_BITS`] of the raw pointer is guaranteed to
    /// be zero. In other words, the pointer is guaranteed to be aligned to
    /// `1 << Self::ALIGN_BITS`.
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

    /// Obtains a shared reference to the original pointer.
    ///
    /// # Safety
    ///
    /// The original pointer must outlive the lifetime parameter `'a`, and during `'a`
    /// no mutable references to the pointer will exist.
    unsafe fn raw_as_ref<'a>(raw: NonNull<Self::Target>) -> Self::Ref<'a>;

    /// Converts a shared reference to a raw pointer.
    fn ref_as_raw(ptr_ref: Self::Ref<'_>) -> NonNull<Self::Target>;
}

/// A type that represents `&'a Box<T>`.
#[derive(Debug)]
pub struct BoxRef<'a, T> {
    inner: *mut T,
    _marker: PhantomData<&'a T>,
}

impl<T> Deref for BoxRef<'_, T> {
    type Target = Box<T>;

    fn deref(&self) -> &Self::Target {
        // SAFETY: A `Box<T>` is guaranteed to be represented by a single pointer [1] and a shared
        // reference to the `Box<T>` during the lifetime `'a` can be created according to the
        // safety requirements of `NonNullPtr::raw_as_ref`.
        //
        // [1]: https://doc.rust-lang.org/std/boxed/#memory-layout
        unsafe { core::mem::transmute(&self.inner) }
    }
}

impl<'a, T> BoxRef<'a, T> {
    /// Dereferences `self` to get a reference to `T` with the lifetime `'a`.
    pub fn deref_target(&self) -> &'a T {
        // SAFETY: The reference is created through `NonNullPtr::raw_as_ref`, hence
        // the original owned pointer and target must outlive the lifetime parameter `'a`,
        // and during `'a` no mutable references to the pointer will exist.
        unsafe { &*(self.inner) }
    }
}

unsafe impl<T: 'static> NonNullPtr for Box<T> {
    type Target = T;

    type Ref<'a>
        = BoxRef<'a, T>
    where
        Self: 'a;

    const ALIGN_BITS: u32 = core::mem::align_of::<T>().trailing_zeros();

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

    unsafe fn raw_as_ref<'a>(raw: NonNull<Self::Target>) -> Self::Ref<'a> {
        BoxRef {
            inner: raw.as_ptr(),
            _marker: PhantomData,
        }
    }

    fn ref_as_raw(ptr_ref: Self::Ref<'_>) -> NonNull<Self::Target> {
        // SAFETY: The pointer representing a `Box` can never be NULL.
        unsafe { NonNull::new_unchecked(ptr_ref.inner) }
    }
}

/// A type that represents `&'a Arc<T>`.
#[derive(Debug)]
pub struct ArcRef<'a, T> {
    inner: ManuallyDrop<Arc<T>>,
    _marker: PhantomData<&'a Arc<T>>,
}

impl<T> Deref for ArcRef<'_, T> {
    type Target = Arc<T>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<'a, T> ArcRef<'a, T> {
    /// Dereferences `self` to get a reference to `T` with the lifetime `'a`.
    pub fn deref_target(&self) -> &'a T {
        // SAFETY: The reference is created through `NonNullPtr::raw_as_ref`, hence
        // the original owned pointer and target must outlive the lifetime parameter `'a`,
        // and during `'a` no mutable references to the pointer will exist.
        unsafe { &*(self.deref().deref() as *const T) }
    }
}

unsafe impl<T: 'static> NonNullPtr for Arc<T> {
    type Target = T;

    type Ref<'a>
        = ArcRef<'a, T>
    where
        Self: 'a;

    const ALIGN_BITS: u32 = core::mem::align_of::<T>().trailing_zeros();

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

    unsafe fn raw_as_ref<'a>(raw: NonNull<Self::Target>) -> Self::Ref<'a> {
        // SAFETY: The safety is upheld by the caller.
        unsafe {
            ArcRef {
                inner: ManuallyDrop::new(Arc::from_raw(raw.as_ptr())),
                _marker: PhantomData,
            }
        }
    }

    fn ref_as_raw(ptr_ref: Self::Ref<'_>) -> NonNull<Self::Target> {
        NonNullPtr::into_raw(ManuallyDrop::into_inner(ptr_ref.inner))
    }
}

/// A type that represents `&'a Weak<T>`.
#[derive(Debug)]
pub struct WeakRef<'a, T> {
    inner: ManuallyDrop<Weak<T>>,
    _marker: PhantomData<&'a Weak<T>>,
}

impl<T> Deref for WeakRef<'_, T> {
    type Target = Weak<T>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

unsafe impl<T: 'static> NonNullPtr for Weak<T> {
    type Target = T;

    type Ref<'a>
        = WeakRef<'a, T>
    where
        Self: 'a;

    // The alignment of `Weak<T>` is 1 instead of `align_of::<T>()`.
    // This is because `Weak::new()` uses a dangling pointer that is _not_ aligned.
    const ALIGN_BITS: u32 = 0;

    fn into_raw(self) -> NonNull<Self::Target> {
        let ptr = Weak::into_raw(self).cast_mut();

        // SAFETY: The pointer representing an `Weak` can never be NULL.
        unsafe { NonNull::new_unchecked(ptr) }
    }

    unsafe fn from_raw(ptr: NonNull<Self::Target>) -> Self {
        let ptr = ptr.as_ptr().cast_const();

        // SAFETY: The safety is upheld by the caller.
        unsafe { Weak::from_raw(ptr) }
    }

    unsafe fn raw_as_ref<'a>(raw: NonNull<Self::Target>) -> Self::Ref<'a> {
        // SAFETY: The safety is upheld by the caller.
        unsafe {
            WeakRef {
                inner: ManuallyDrop::new(Weak::from_raw(raw.as_ptr())),
                _marker: PhantomData,
            }
        }
    }

    fn ref_as_raw(ptr_ref: Self::Ref<'_>) -> NonNull<Self::Target> {
        NonNullPtr::into_raw(ManuallyDrop::into_inner(ptr_ref.inner))
    }
}
