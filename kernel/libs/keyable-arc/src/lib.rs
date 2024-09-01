// SPDX-License-Identifier: MPL-2.0

//! Same as the standard `Arc`, except that it can be used as the key type of a hash table.
//!
//! # Motivation
//!
//! A type `K` is _keyable_ if it can be used as the key type for a hash map. Specifically,
//! according to the document of `std::collections::HashMap`, the type `K` must satisfy
//! the following properties.
//!
//! 1. It implements the `Eq` and `Hash` traits.
//! 2. The two values of `k1` and `k2` of type `K` equal to each other,
//!    if and only if their hash values equal to each other.
//! 3. The hashes of a value of `k` of type `K` cannot change while it
//!    is in a map.
//!
//! Sometimes we want to use `Arc<T>` as the key type for a hash map but cannot do so
//! since `T` does not satisfy the properties above. For example, a lot of types
//! do not or cannot implemennt the `Eq` trait. This is when `KeyableArc<T>` can come
//! to your aid.
//!
//! # Overview
//!
//! For any type `T`, `KeyableArc<T>` satisfies all the properties to be keyable.
//! This can be achieved easily and efficiently as we can simply use the address
//! of the data (of `T` type) of a `KeyableArc<T>` object in the heap to determine the
//! equality and hash of the `KeyableArc<T>` object. As the address won't change for
//! an immutable `KeyableArc<T>` object, the hash and equality also stay the same.
//!
//! This crate is `#[no_std]` compatible, but requires the `alloc` crate.
//!
//! # Usage
//!
//! Here is a basic example to how that `KeyableArc<T>` is keyable even when `T`
//! is not.
//!
//! ```rust
//! use std::collections::HashMap;
//! use std::sync::Arc;
//! use keyable_arc::KeyableArc;
//!
//! struct Dummy; // Does not implement Eq and Hash
//!
//! let map: HashMap<KeyableArc<Dummy>, String> = HashMap::new();
//! ```
//!
//! `KeyableArc` is a reference counter-based smart pointer, just like `Arc`.
//! So you can use `KeyableArc` the same way you would use `Arc`.
//!
//! ```rust
//! use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
//! use keyable_arc::KeyableArc;
//!
//! let key_arc0 = KeyableArc::new(AtomicU64::new(0));
//! let key_arc1 = key_arc0.clone();
//! assert!(key_arc0.load(Relaxed) == 0 && key_arc1.load(Relaxed) == 0);
//!
//! key_arc0.fetch_add(1, Relaxed);
//! assert!(key_arc0.load(Relaxed) == 1 && key_arc1.load(Relaxed) == 1);
//! ```
//!
//! # Differences from `Arc<T>`
//!
//! Notice how `KeyableArc` differs from standard smart pointers in determining equality?
//! Two `KeyableArc` objects are considered different even when their data have the same
//! value.
//!
//! ```rust
//! use keyable_arc::KeyableArc;
//!
//! let key_arc0 = KeyableArc::new(0);
//! let key_arc1 = key_arc0.clone();
//! assert!(key_arc0 == key_arc1);
//! assert!(*key_arc0 == *key_arc1);
//!
//! let key_arc1 = KeyableArc::new(0);
//! assert!(key_arc0 != key_arc1);
//! assert!(*key_arc0 == *key_arc1);
//! ```
//!
//! `KeyableArc<T>` is simply a wrapper of `Arc<T>. So converting between them
//! through the `From` and `Into` traits is zero cost.
//!
//! ```rust
//! use std::sync::Arc;
//! use keyable_arc::KeyableArc;
//!
//! let key_arc: KeyableArc<u32> = Arc::new(0).into();
//! let arc: Arc<u32> = KeyableArc::new(0).into();
//! ```
//!
//! # The weak version
//!
//! `KeyableWeak<T>` is the weak version of `KeyableArc<T>`, just like `Weak<T>` is
//! that of `Arc<T>`. And of course, `KeyableWeak<T>` is also _keyable_ for any
//! type `T`.

// TODO: Add `KeyableBox<T>` or other keyable versions of smart pointers.
// If this is needed in the future, this crate should be renamed to `keyable`.

// TODO: Add the missing methods offered by `Arc` or `Weak` but not their
// keyable counterparts.

#![cfg_attr(not(test), no_std)]
#![feature(coerce_unsized)]
#![feature(unsize)]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::sync::{Arc, Weak};
use core::{
    borrow::Borrow,
    cmp::Ordering,
    fmt,
    hash::{Hash, Hasher},
    marker::Unsize,
    ops::{CoerceUnsized, Deref},
};

/// Same as the standard `Arc`, except that it can be used as the key type of a hash table.
#[repr(transparent)]
pub struct KeyableArc<T: ?Sized>(Arc<T>);

impl<T> KeyableArc<T> {
    /// Constructs a new instance of `KeyableArc<T>`.
    #[inline]
    pub fn new(data: T) -> Self {
        Self(Arc::new(data))
    }
}

impl<T: ?Sized> KeyableArc<T> {
    /// Returns a raw pointer to the object `T` pointed to by this `KeyableArc<T>`.
    #[inline]
    pub fn as_ptr(this: &Self) -> *const T {
        Arc::as_ptr(&this.0)
    }

    /// Creates a new `KeyableWeak` pointer to this allocation.
    pub fn downgrade(this: &Self) -> KeyableWeak<T> {
        Arc::downgrade(&this.0).into()
    }
}

impl<T: ?Sized> Deref for KeyableArc<T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T: ?Sized> AsRef<T> for KeyableArc<T> {
    #[inline]
    fn as_ref(&self) -> &T {
        self
    }
}

impl<T: ?Sized> Borrow<T> for KeyableArc<T> {
    #[inline]
    fn borrow(&self) -> &T {
        self
    }
}

impl<T: ?Sized> From<Arc<T>> for KeyableArc<T> {
    #[inline]
    fn from(arc: Arc<T>) -> Self {
        Self(arc)
    }
}

impl<T: ?Sized> From<KeyableArc<T>> for Arc<T> {
    #[inline]
    fn from(value: KeyableArc<T>) -> Self {
        value.0
    }
}

impl<T: ?Sized> PartialEq for KeyableArc<T> {
    fn eq(&self, other: &Self) -> bool {
        core::ptr::eq(Arc::as_ptr(&self.0), Arc::as_ptr(&other.0))
    }
}

impl<T: ?Sized> Eq for KeyableArc<T> {}

impl<T: ?Sized> Ord for KeyableArc<T> {
    fn cmp(&self, other: &Self) -> Ordering {
        Arc::as_ptr(&self.0)
            .cast::<()>()
            .cmp(&Arc::as_ptr(&other.0).cast::<()>())
    }
}

impl<T: ?Sized> PartialOrd for KeyableArc<T> {
    fn partial_cmp(&self, other: &KeyableArc<T>) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<T: ?Sized> Hash for KeyableArc<T> {
    fn hash<H: Hasher>(&self, s: &mut H) {
        Arc::as_ptr(&self.0).hash(s)
    }
}

impl<T: ?Sized> Clone for KeyableArc<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T: ?Sized + fmt::Debug> fmt::Debug for KeyableArc<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&**self, f)
    }
}

//=========================================================
// The weak version
//=========================================================

/// The weak counterpart of `KeyableArc<T>`, similar to `Weak<T>`.
///
/// `KeyableWeak<T>` is also _keyable_ for any type `T` just like
/// `KeyableArc<T>`.
#[repr(transparent)]
pub struct KeyableWeak<T: ?Sized>(Weak<T>);

impl<T> KeyableWeak<T> {
    /// Constructs a new `KeyableWeak<T>`, without allocating any memory.
    /// Calling `upgrade` on the return value always gives `None`.
    #[inline]
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self(Weak::new())
    }

    /// Returns a raw pointer to the object `T` pointed to by this `KeyableWeak<T>`.
    ///
    /// The pointer is valid only if there are some strong references.
    /// The pointer may be dangling, unaligned or even null otherwise.
    #[inline]
    pub fn as_ptr(&self) -> *const T {
        self.0.as_ptr()
    }
}

impl<T: ?Sized> KeyableWeak<T> {
    /// Attempts to upgrade the Weak pointer to an Arc,
    /// delaying dropping of the inner value if successful.
    ///
    /// Returns None if the inner value has since been dropped.
    #[inline]
    pub fn upgrade(&self) -> Option<KeyableArc<T>> {
        self.0.upgrade().map(|arc| arc.into())
    }

    /// Gets the number of strong pointers pointing to this allocation.
    #[inline]
    pub fn strong_count(&self) -> usize {
        self.0.strong_count()
    }

    /// Gets the number of weak pointers pointing to this allocation.
    #[inline]
    pub fn weak_count(&self) -> usize {
        self.0.weak_count()
    }
}

impl<T: ?Sized> PartialEq for KeyableWeak<T> {
    fn eq(&self, other: &Self) -> bool {
        core::ptr::eq(self.0.as_ptr(), other.0.as_ptr())
    }
}

impl<T: ?Sized> Eq for KeyableWeak<T> {}

impl<T: ?Sized> Ord for KeyableWeak<T> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0
            .as_ptr()
            .cast::<()>()
            .cmp(&other.0.as_ptr().cast::<()>())
    }
}

impl<T: ?Sized> PartialOrd for KeyableWeak<T> {
    fn partial_cmp(&self, other: &KeyableWeak<T>) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<T: ?Sized> Hash for KeyableWeak<T> {
    fn hash<H: Hasher>(&self, s: &mut H) {
        self.0.as_ptr().hash(s)
    }
}

impl<T: ?Sized> From<Weak<T>> for KeyableWeak<T> {
    #[inline]
    fn from(weak: Weak<T>) -> Self {
        Self(weak)
    }
}

impl<T: ?Sized> From<KeyableWeak<T>> for Weak<T> {
    fn from(value: KeyableWeak<T>) -> Self {
        value.0
    }
}

impl<T: ?Sized> Clone for KeyableWeak<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T: ?Sized + fmt::Debug> fmt::Debug for KeyableWeak<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(KeyableWeak)")
    }
}

// Enabling type coercing, e.g., converting from `KeyableArc<T>` to `KeyableArc<dyn S>`,
// where `T` implements `S`.
impl<T: ?Sized + Unsize<U>, U: ?Sized> CoerceUnsized<KeyableArc<U>> for KeyableArc<T> {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downgrade_and_upgrade() {
        let arc = KeyableArc::new(1);
        let weak = KeyableArc::downgrade(&arc);
        assert!(arc.clone() == weak.upgrade().unwrap());
        assert!(weak == KeyableArc::downgrade(&arc));
    }

    #[test]
    fn debug_format() {
        println!("{:?}", KeyableArc::new(1u32));
        println!("{:?}", KeyableWeak::<u32>::new());
    }

    #[test]
    fn use_as_key() {
        use std::collections::HashMap;

        let mut map: HashMap<KeyableArc<u32>, u32> = HashMap::new();
        let key = KeyableArc::new(1);
        let val = 1;
        map.insert(key.clone(), val);
        assert!(map.get(&key) == Some(&val));
        assert!(map.remove(&key) == Some(val));
        assert!(map.keys().count() == 0);
    }

    #[test]
    fn as_trait_object() {
        trait DummyTrait {}
        struct DummyStruct;
        impl DummyTrait for DummyStruct {}

        let arc_struct = KeyableArc::new(DummyStruct);
        let arc_dyn0: KeyableArc<dyn DummyTrait> = arc_struct.clone();
        let arc_dyn1: KeyableArc<dyn DummyTrait> = arc_struct.clone();
        assert!(arc_dyn0 == arc_dyn1);
    }
}
