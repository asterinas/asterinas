// SPDX-License-Identifier: MPL-2.0

//! This module provides a trait and some auxiliary types to help abstract and
//! work with non-null pointers.

use alloc::sync::Weak;
use core::{marker::PhantomData, mem::ManuallyDrop, ops::Deref, ptr::NonNull};

use crate::prelude::*;

/// A trait that abstracts pointers that are non-null.
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
pub unsafe trait NonNullPtr: Send + 'static {
    /// A type that behaves just like a shared references to the `NonNullPtr`.
    type Ref<'a>: Deref<Target = Self>
    where
        Self: 'a;

    /// Converts to a raw pointer.
    ///
    /// Each call to `into_raw` must be paired with a call to `from_raw`
    /// in order to avoid memory leakage.
    ///
    /// The resulting raw pointer must be valid to be immutably accessed
    /// or borrowed until `from_raw` is called.
    fn into_raw(self) -> NonNull<()>;

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
    unsafe fn from_raw(ptr: NonNull<()>) -> Self;

    /// Obtains a shared reference to the original pointer.
    ///
    /// # Safety
    ///
    /// The original pointer must outlive the lifetime parameter `'a`, and during `'a`
    /// no mutable references to the pointer will exist.
    unsafe fn raw_as_ref<'a>(raw: NonNull<()>) -> Self::Ref<'a>;

    /// Converts a shared reference to a pointer.
    ///
    /// # Panic
    ///
    /// If the input reference is not created by [`raw_as_ref`] and is a reference to
    /// null pointer, this method will panic.
    fn ref_as_raw(ptr_ref: Self::Ref<'_>) -> NonNull<()>;
}

/// A trait that abstracts a reference to the pointer that have the ownership of the
/// objects they refer to.
///
/// The target that the original pointer refers to can be read with the lifetime `'a`.
///
/// # Safety
///
/// This reference should only be created through [NonNullPtr::raw_as_ref].
pub unsafe trait OwnedPtrRef<'a>: Deref<Target = Self::RefPtr> {
    /// The target type that the original pointer refers to.
    type OwnedTarget;
    /// The referenced pointer.
    type RefPtr: NonNullPtr + Deref<Target = Self::OwnedTarget>;

    /// Borrows self as a reference to `OwnedTarget` with the lifetime `'a`.
    fn read_target(&self) -> &'a Self::OwnedTarget {
        // SAFETY: The reference is created through `NonNullPtr::raw_as_ref`, hence
        // the original owned pointer and target must outlive the lifetime parameter `'a`,
        // and during `'a` no mutable references to the pointer will exist.
        unsafe { &*(self.deref().deref() as *const Self::OwnedTarget) }
    }
}

/// A type that represents `&'a Box<T>`.
#[derive(PartialEq, Debug)]
pub struct BoxRef<'a, T: Send + 'static> {
    inner: *mut T,
    _marker: PhantomData<&'a T>,
}

impl<T: Send + 'static> Deref for BoxRef<'_, T> {
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

// SAFETY: `BoxRef<T>` can only be created through `NonNullPtr::raw_as_ref`.
unsafe impl<'a, T: Send + 'static> OwnedPtrRef<'a> for BoxRef<'a, T> {
    type OwnedTarget = T;
    type RefPtr = Box<T>;
}

unsafe impl<T: Send + 'static> NonNullPtr for Box<T> {
    type Ref<'a>
        = BoxRef<'a, T>
    where
        Self: 'a;

    fn into_raw(self) -> NonNull<()> {
        let ptr = Box::into_raw(self).cast();

        // SAFETY: The pointer representing a `Box` can never be NULL.
        unsafe { NonNull::new_unchecked(ptr) }
    }

    unsafe fn from_raw(ptr: NonNull<()>) -> Self {
        let ptr = ptr.as_ptr().cast();

        // SAFETY: The safety is upheld by the caller.
        unsafe { Box::from_raw(ptr) }
    }

    unsafe fn raw_as_ref<'a>(raw: NonNull<()>) -> Self::Ref<'a> {
        BoxRef {
            inner: raw.as_ptr().cast(),
            _marker: PhantomData,
        }
    }

    fn ref_as_raw(ptr_ref: Self::Ref<'_>) -> NonNull<()> {
        NonNull::new(ptr_ref.inner.cast()).unwrap()
    }
}

/// A type that represents `&'a Arc<T>`.
#[derive(PartialEq, Debug)]
pub struct ArcRef<'a, T: Send + Sync + 'static> {
    inner: ManuallyDrop<Arc<T>>,
    _marker: PhantomData<&'a Arc<T>>,
}

impl<T: Send + Sync + 'static> Deref for ArcRef<'_, T> {
    type Target = Arc<T>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

// SAFETY: `ArcRef<T>` can only be created through `NonNullPtr::raw_as_ref`.
unsafe impl<'a, T: Send + Sync + 'static> OwnedPtrRef<'a> for ArcRef<'a, T> {
    type OwnedTarget = T;
    type RefPtr = Arc<T>;
}

unsafe impl<T: Send + Sync + 'static> NonNullPtr for Arc<T> {
    type Ref<'a>
        = ArcRef<'a, T>
    where
        Self: 'a;

    fn into_raw(self) -> NonNull<()> {
        let ptr = Arc::into_raw(self).cast_mut().cast();

        // SAFETY: The pointer representing an `Arc` can never be NULL.
        unsafe { NonNull::new_unchecked(ptr) }
    }

    unsafe fn from_raw(ptr: NonNull<()>) -> Self {
        let ptr = ptr.as_ptr().cast_const().cast();

        // SAFETY: The safety is upheld by the caller.
        unsafe { Arc::from_raw(ptr) }
    }

    unsafe fn raw_as_ref<'a>(raw: NonNull<()>) -> Self::Ref<'a> {
        // SAFETY: By the safety requirements of `NonNullPtr::raw_as_ref`, the original pointer
        // outlives the lifetime parameter `'a` and during `'a` no mutable references to it can
        // exist. Thus, a shared reference to the original pointer can be created.
        unsafe {
            ArcRef {
                inner: ManuallyDrop::new(Arc::from_raw(raw.as_ptr().cast())),
                _marker: PhantomData,
            }
        }
    }

    fn ref_as_raw(ptr_ref: Self::Ref<'_>) -> NonNull<()> {
        let raw_ptr = Arc::into_raw(ManuallyDrop::into_inner(ptr_ref.inner))
            .cast_mut()
            .cast();
        NonNull::new(raw_ptr).unwrap()
    }
}

/// A type that represents `&'a Weak<T>`.
#[derive(Debug)]
pub struct WeakRef<'a, T: Send + Sync + 'static> {
    inner: ManuallyDrop<Weak<T>>,
    _marker: PhantomData<&'a Weak<T>>,
}

impl<T: Send + Sync + 'static> Deref for WeakRef<'_, T> {
    type Target = Weak<T>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

unsafe impl<T: Send + Sync + 'static> NonNullPtr for Weak<T> {
    type Ref<'a>
        = WeakRef<'a, T>
    where
        Self: 'a;

    fn into_raw(self) -> NonNull<()> {
        let ptr = Weak::into_raw(self).cast_mut().cast();
        // SAFETY: The pointer representing an `Weak` can never be NULL.
        unsafe { NonNull::new_unchecked(ptr) }
    }

    unsafe fn from_raw(ptr: NonNull<()>) -> Self {
        let ptr = ptr.as_ptr().cast_const().cast();

        // SAFETY: The safety is upheld by the caller.
        unsafe { Weak::from_raw(ptr) }
    }

    unsafe fn raw_as_ref<'a>(raw: NonNull<()>) -> Self::Ref<'a> {
        // SAFETY: By the safety requirements of `NonNullPtr::raw_as_ref`, the original pointer
        // outlives the lifetime parameter `'a` and during `'a` no mutable references to it can
        // exist. Thus, a shared reference to the original pointer can be created.
        unsafe {
            WeakRef {
                inner: ManuallyDrop::new(Weak::from_raw(raw.as_ptr().cast())),
                _marker: PhantomData,
            }
        }
    }

    fn ref_as_raw(ptr_ref: Self::Ref<'_>) -> NonNull<()> {
        let raw_ptr = Weak::<T>::into_raw(ManuallyDrop::into_inner(ptr_ref.inner))
            .cast_mut()
            .cast();
        NonNull::new(raw_ptr).unwrap()
    }
}
