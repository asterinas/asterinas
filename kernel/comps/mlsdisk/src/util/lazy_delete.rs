// SPDX-License-Identifier: MPL-2.0

use core::{
    fmt,
    ops::{Deref, DerefMut},
    sync::atomic::{AtomicBool, Ordering},
};

use crate::prelude::*;

/// An object that may be deleted lazily.
///
/// Lazy-deletion is a technique to postpone the real deletion of an object.
/// This technique allows an object to remain usable even after a decision
/// to delete the object has been made. Of course. After the "real" deletion
/// is carried out, the object will no longer be usable.
///
/// A classic example is file deletion in UNIX file systems.
///
/// ```ignore
/// int fd = open("path/to/my_file", O_RDONLY);
/// unlink("path/to/my_file");
/// // fd is still valid after unlink
/// ```
///
/// `LazyDelete<T>` enables lazy deletion of any object of `T`.
/// Here is a simple example.
///
/// ```
/// use crate::util::LazyDelete;
///
/// let lazy_delete_u32 = LazyDelete::new(123_u32, |obj| {
///     println!("the real deletion happens in this closure");
/// });
///
/// // The object is still usable after it is deleted (lazily)
/// LazyDelete::delete(&lazy_delete_u32);
/// assert!(*lazy_delete_u32 == 123);
///
/// // The deletion operation will be carried out when it is dropped
/// drop(lazy_delete_u32);
/// ```
#[expect(clippy::type_complexity)]
pub struct LazyDelete<T> {
    obj: T,
    is_deleted: AtomicBool,
    delete_fn: Option<Box<dyn FnOnce(&mut T) + Send + Sync>>,
}

impl<T: fmt::Debug> fmt::Debug for LazyDelete<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LazyDelete")
            .field("obj", &self.obj)
            .field("is_deleted", &Self::is_deleted(self))
            .finish()
    }
}

impl<T> LazyDelete<T> {
    /// Creates a new instance of `LazyDelete`.
    ///
    /// The `delete_fn` will be called only if this instance of `LazyDelete` is
    /// marked deleted by the `delete` method and only when this instance
    /// of `LazyDelete` is dropped.
    pub fn new<F: FnOnce(&mut T) + Send + Sync + 'static>(obj: T, delete_fn: F) -> Self {
        Self {
            obj,
            is_deleted: AtomicBool::new(false),
            delete_fn: Some(Box::new(delete_fn) as _),
        }
    }

    /// Mark this instance deleted.
    pub fn delete(this: &Self) {
        this.is_deleted.store(true, Ordering::Release);
    }

    /// Returns whether this instance has been marked deleted.
    pub fn is_deleted(this: &Self) -> bool {
        this.is_deleted.load(Ordering::Acquire)
    }
}

impl<T> Deref for LazyDelete<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.obj
    }
}

impl<T> DerefMut for LazyDelete<T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.obj
    }
}

impl<T> Drop for LazyDelete<T> {
    fn drop(&mut self) {
        if Self::is_deleted(self) {
            let delete_fn = self.delete_fn.take().unwrap();
            (delete_fn)(&mut self.obj);
        }
    }
}
