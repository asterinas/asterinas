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

/// Creates a pointer whose type matches the expression, but whose value is always NULL.
///
/// This is a helper macro, typically used in another macro to help with type inference.
///
/// The expression is guaranteed never to be executed, so it can contain arbitrarily unsafe code
/// without causing any soundness problems.
#[macro_export]
macro_rules! ptr_null_of {
    ($expr:expr $(,)?) => {
        if true {
            core::ptr::null()
        } else {
            unreachable!();

            // SAFETY: This is dead code and will never be executed.
            //
            // One may wonder: is it possible for the dead code to
            // trigger UBs by simply being compiled, rather than being executed?
            // More specifically, what if the caller attempts to
            // trick the macro into defining unsafe language items,
            // like static variables, functions, implementation blocks, or attributes,
            // those that are not executed.
            // Luckily for us, in such cases, the Rust compiler would complain that
            // "items do not inherit unsafety from separate enclosing items".
            #[expect(unreachable_code)]
            unsafe {
                $expr
            }
        }
    };
}
