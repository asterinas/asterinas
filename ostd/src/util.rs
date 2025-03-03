// SPDX-License-Identifier: MPL-2.0

/// Asserts that a boolean expression is `true` at compile-time.
///
/// Rust provides [`const` blocks], which can be used flexibly within methods, but cannot be used
/// directly at the top level. This macro serves as a helper to perform compile-time assertions
/// outside of methods.
///
/// [`const` blocks]: https://doc.rust-lang.org/reference/expressions/block-expr.html#const-blocks
//
// TODO: Introduce `const_assert_eq!()` once `assert_eq!()` can be used in the `const` context.
#[macro_export]
macro_rules! const_assert {
    ($cond:expr $(,)?) => { const _: () = assert!($cond); };
    ($cond:expr, $($arg:tt)+) => { const _: () = assert!($cond, $($arg)*); };
}

/// A marker trait that represents a type has the same size as `T`.
///
/// # Safety
///
/// Types that implement `SameSizeAs<T>` must have the same size as `T`.
pub unsafe trait SameSizeAs<T> {}
