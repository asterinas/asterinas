// SPDX-License-Identifier: MPL-2.0

use alloc::fmt;

/// Always [`Sync`], but unsafe to reference the data.
pub(crate) struct ForceSync<T>(T);

impl<T> fmt::Debug for ForceSync<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ForceSync").finish_non_exhaustive()
    }
}

// SAFETY: The caller of the `ForceSync::get` method must ensure that the underlying data is not
// concurrently accessed if the underlying type is not `Sync`.
unsafe impl<T> Sync for ForceSync<T> {}

impl<T> ForceSync<T> {
    /// Creates an instance with `data` as the inner data.
    pub(crate) fn new(data: T) -> Self {
        Self(data)
    }

    /// Returns a reference to the inner data.
    ///
    /// # Safety
    ///
    /// If the data type is not [`Sync`], the caller must ensure that the data is not accessed
    /// concurrently.
    pub(crate) unsafe fn get(&self) -> &T {
        &self.0
    }
}

impl<T: Clone> Clone for ForceSync<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T: Default> Default for ForceSync<T> {
    fn default() -> Self {
        Self(T::default())
    }
}
