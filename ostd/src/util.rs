// SPDX-License-Identifier: MPL-2.0

/// A marker trait that represents a type has the same size as `T`.
///
/// # Safety
///
/// Types that implement `SameSizeAs<T>` must have the same size as `T`.
pub unsafe trait SameSizeAs<T> {}
