// SPDX-License-Identifier: MPL-2.0

/// This trait is a _fallible_ version of `Clone`.
///
/// If any object of a type `T` is duplicable, then `T` should implement
/// `Clone`. However, if whether an object is duplicable must be determined
/// on a per-object basis at runtime, then `T` should implement `Dup` as
/// the `dup` method is allowed to return an error.
///
/// As a best practice, the `Clone` and `Dup` traits should be implemented
/// _exclusively_ to one another. In other words, a type should not implement
/// both traits.
pub trait Dup: Sized {
    fn dup(&self) -> ostd::Result<Self>;
}
