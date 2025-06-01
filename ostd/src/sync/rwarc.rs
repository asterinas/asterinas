// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;
use core::sync::atomic::{fence, AtomicUsize, Ordering};

use super::{PreemptDisabled, RwLock, RwLockReadGuard, RwLockWriteGuard};

/// A reference-counting pointer with read-write capabilities.
///
/// This is essentially `Arc<RwLock<T>>`, so it can provide read-write capabilities through
/// [`RwArc::read`] and [`RwArc::write`].
///
/// In addition, this allows to derive another reference-counting pointer with read-only
/// capabilities ([`RoArc`]) via [`RwArc::clone_ro`].
///
/// The purpose of having this type is to allow lockless (read) access to the underlying data when
/// there is only one [`RwArc`] instance for the particular allocation (note that there can be any
/// number of [`RoArc`] instances for that allocation). See the [`RwArc::get`] method for more
/// details.
pub struct RwArc<T>(Arc<Inner<T>>);

/// A reference-counting pointer with read-only capabilities.
///
/// This type can be created from an existing [`RwArc`] using its [`RwArc::clone_ro`] method. See
/// the type and method documentation for more details.
pub struct RoArc<T>(Arc<Inner<T>>);

struct Inner<T> {
    data: RwLock<T>,
    num_rw: AtomicUsize,
}

impl<T> RwArc<T> {
    /// Creates a new `RwArc<T>`.
    pub fn new(data: T) -> Self {
        let inner = Inner {
            data: RwLock::new(data),
            num_rw: AtomicUsize::new(1),
        };
        Self(Arc::new(inner))
    }

    /// Acquires the read lock for immutable access.
    pub fn read(&self) -> RwLockReadGuard<T, PreemptDisabled> {
        self.0.data.read()
    }

    /// Acquires the write lock for mutable access.
    pub fn write(&self) -> RwLockWriteGuard<T, PreemptDisabled> {
        self.0.data.write()
    }

    /// Returns an immutable reference if no other `RwArc` points to the same allocation.
    ///
    /// This method is cheap because it does not acquire a lock.
    ///
    /// It's still sound because:
    /// - The mutable reference to `self` and the condition ensure that we are exclusively
    ///   accessing the unique `RwArc` instance for the particular allocation.
    /// - There may be any number of [`RoArc`]s pointing to the same allocation, but they may only
    ///   produce immutable references to the underlying data.
    pub fn get(&mut self) -> Option<&T> {
        if self.0.num_rw.load(Ordering::Relaxed) > 1 {
            return None;
        }

        // This will synchronize with `RwArc::drop` to make sure its changes are visible to us.
        fence(Ordering::Acquire);

        let data_ptr = self.0.data.as_ptr();

        // SAFETY: The data is valid. During the lifetime, no one will be able to create a mutable
        // reference to the data, so it's okay to create an immutable reference like the one below.
        Some(unsafe { &*data_ptr })
    }

    /// Clones a [`RoArc`] that points to the same allocation.
    pub fn clone_ro(&self) -> RoArc<T> {
        RoArc(self.0.clone())
    }
}

impl<T> Clone for RwArc<T> {
    fn clone(&self) -> Self {
        let inner = self.0.clone();

        // Note that overflowing the counter will make it unsound. But not to worry: the above
        // `Arc::clone` must have already aborted the kernel before this happens.
        inner.num_rw.fetch_add(1, Ordering::Relaxed);

        Self(inner)
    }
}

impl<T> Drop for RwArc<T> {
    fn drop(&mut self) {
        self.0.num_rw.fetch_sub(1, Ordering::Release);
    }
}

impl<T: Clone> RwArc<T> {
    /// Returns the contained value by cloning it.
    pub fn get_cloned(&self) -> T {
        let guard = self.read();
        guard.clone()
    }
}

impl<T> RoArc<T> {
    /// Acquires the read lock for immutable access.
    pub fn read(&self) -> RwLockReadGuard<T, PreemptDisabled> {
        self.0.data.read()
    }
}

#[cfg(ktest)]
mod test {
    use super::*;
    use crate::prelude::*;

    #[ktest]
    fn lockless_get() {
        let mut rw1 = RwArc::new(1u32);
        assert_eq!(rw1.get(), Some(1).as_ref());

        let _ro = rw1.clone_ro();
        assert_eq!(rw1.get(), Some(1).as_ref());

        let rw2 = rw1.clone();
        assert_eq!(rw1.get(), None);

        drop(rw2);
        assert_eq!(rw1.get(), Some(1).as_ref());
    }
}
