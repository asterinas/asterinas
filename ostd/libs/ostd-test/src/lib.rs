// SPDX-License-Identifier: MPL-2.0

//! # The kernel mode testing framework of OSTD.
//!
//! `ostd-test` stands for kernel-mode testing framework for OSTD. Its goal is to provide a
//! `cargo test`-like experience for any `#![no_std]` bare metal crates.
//!
//! In OSTD, all the tests written in the source tree of the crates will be run
//! immediately after the initialization of `ostd`. Thus you can use any
//! feature provided by the frame including the heap allocator, etc.
//!
//! By all means, ostd-test is an individual crate that only requires:
//!  - a custom linker script section `.ktest_array`,
//!  - and an alloc implementation.
//!
//! And the OSTD happens to provide both of them. Thus, any crates depending
//! on the OSTD can use ostd-test without any extra dependency.
//!
//! ## Usage
//!
//! To write a unit test for any crates, it is recommended to create a new test
//! module, e.g.:
//!
//! ```rust
//! #[cfg(ktest)]
//! mod test {
//!     use ostd::prelude::*;
//!
//!     #[ktest]
//!     fn trivial_assertion() {
//!         assert_eq!(0, 0);
//!     }
//!     #[ktest]
//!     #[should_panic]
//!     fn failing_assertion() {
//!         assert_eq!(0, 1);
//!     }
//!     #[ktest]
//!     #[should_panic(expected = "expected panic message")]
//!     fn expect_panic() {
//!         panic!("expected panic message");
//!     }
//! }
//! ```
//!
//! Any crates using the ostd-test framework should be linked with ostd.
//!
//! By the way, `#[ktest]` attribute along also works, but it hinders test control
//! using cfgs since plain attribute marked test will be executed in all test runs
//! no matter what cfgs are passed to the compiler. More importantly, using `#[ktest]`
//! without cfgs occupies binary real estate since the `.ktest_array` section is not
//! explicitly stripped in normal builds.
//!
//! Rust cfg is used to control the compilation of the test module. In cooperation
//! with the `ktest` framework, OSDK will set the `RUSTFLAGS` environment variable
//! to pass the cfgs to all rustc invocations. To run the tests, you simply need
//! to use the command `cargo osdk test` in the crate directory. For more information,
//! please refer to the OSDK documentation.
//!
//! We support the `#[should_panic]` attribute just in the same way as the standard
//! library do, but the implementation is quite slow currently. Use it with cautious.
//!
//! Doctest is not taken into consideration yet, and the interface is subject to
//! change.
//!

#![cfg_attr(not(test), no_std)]

extern crate alloc;
use alloc::{boxed::Box, string::String};

#[derive(Clone, Debug)]
pub struct PanicInfo {
    pub message: String,
    pub file: String,
    pub line: usize,
    pub col: usize,
}

impl core::fmt::Display for PanicInfo {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        writeln!(f, "Panicked at {}:{}:{}", self.file, self.line, self.col)?;
        writeln!(f, "{}", self.message)
    }
}

/// The error that may occur during the test.
#[derive(Clone)]
pub enum KtestError {
    Panic(Box<PanicInfo>),
    ShouldPanicButNoPanic,
    ExpectedPanicNotMatch(&'static str, Box<PanicInfo>),
    Unknown,
}

/// The information of the unit test.
#[derive(Clone, PartialEq, Debug)]
pub struct KtestItemInfo {
    /// The path of the module, not including the function name.
    ///
    /// It would be separated by `::`.
    pub module_path: &'static str,
    /// The name of the unit test function.
    pub fn_name: &'static str,
    /// The name of the crate.
    pub package: &'static str,
    /// The source file where the test function resides.
    pub source: &'static str,
    /// The line number of the test function in the file.
    pub line: usize,
    /// The column number of the test function in the file.
    pub col: usize,
}

#[derive(Clone, PartialEq, Debug)]
pub struct KtestItem {
    fn_: fn() -> (),
    should_panic: (bool, Option<&'static str>),
    info: KtestItemInfo,
}

type CatchUnwindImpl = fn(f: fn() -> ()) -> Result<(), Box<dyn core::any::Any + Send>>;

impl KtestItem {
    /// Create a new [`KtestItem`].
    ///
    /// Do not use this function directly. Instead, use the `#[ktest]`
    /// attribute to mark the test function.
    #[doc(hidden)]
    pub const fn new(
        fn_: fn() -> (),
        should_panic: (bool, Option<&'static str>),
        info: KtestItemInfo,
    ) -> Self {
        Self {
            fn_,
            should_panic,
            info,
        }
    }

    /// Get the information of the test.
    pub fn info(&self) -> &KtestItemInfo {
        &self.info
    }

    /// Run the test with a given catch_unwind implementation.
    pub fn run(&self, catch_unwind_impl: &CatchUnwindImpl) -> Result<(), KtestError> {
        let test_result = catch_unwind_impl(self.fn_);
        if !self.should_panic.0 {
            // Should not panic.
            match test_result {
                Ok(()) => Ok(()),
                Err(e) => match e.downcast::<PanicInfo>() {
                    Ok(s) => Err(KtestError::Panic(s)),
                    Err(_payload) => Err(KtestError::Unknown),
                },
            }
        } else {
            // Should panic.
            match test_result {
                Ok(()) => Err(KtestError::ShouldPanicButNoPanic),
                Err(e) => match e.downcast::<PanicInfo>() {
                    Ok(s) => {
                        if let Some(expected) = self.should_panic.1 {
                            if s.message == expected {
                                Ok(())
                            } else {
                                Err(KtestError::ExpectedPanicNotMatch(expected, s))
                            }
                        } else {
                            Ok(())
                        }
                    }
                    Err(_payload) => Err(KtestError::Unknown),
                },
            }
        }
    }
}

macro_rules! ktest_array {
    () => {{
        extern "C" {
            fn __ktest_array();
            fn __ktest_array_end();
        }
        let item_size = core::mem::size_of::<KtestItem>();
        let l = (__ktest_array_end as usize - __ktest_array as usize) / item_size;
        // SAFETY: __ktest_array is a static section consisting of KtestItem.
        unsafe { core::slice::from_raw_parts(__ktest_array as *const KtestItem, l) }
    }};
}

/// The iterator of the ktest array.
pub struct KtestIter {
    index: usize,
}

impl Default for KtestIter {
    fn default() -> Self {
        Self::new()
    }
}

impl KtestIter {
    /// Create a new [`KtestIter`].
    ///
    /// It will iterate over all the tests (marked with `#[ktest]`).
    pub fn new() -> Self {
        Self { index: 0 }
    }
}

impl core::iter::Iterator for KtestIter {
    type Item = KtestItem;

    fn next(&mut self) -> Option<Self::Item> {
        let ktest_item = ktest_array!().get(self.index)?;
        self.index += 1;
        Some(ktest_item.clone())
    }
}

// The whitelists that will be generated by the OSDK as static consts.
// They deliver the target tests that the user wants to run.
extern "Rust" {
    static KTEST_TEST_WHITELIST: Option<&'static [&'static str]>;
    static KTEST_CRATE_WHITELIST: Option<&'static [&'static str]>;
}

/// Get the whitelist of the tests.
///
/// The whitelist is generated by the OSDK runner, indicating name of the
/// target tests that the user wants to run.
pub fn get_ktest_test_whitelist() -> Option<&'static [&'static str]> {
    // SAFETY: The two extern statics in the base crate are generated by OSDK.
    unsafe { KTEST_TEST_WHITELIST }
}

/// Get the whitelist of the crates.
///
/// The whitelist is generated by the OSDK runner, indicating the target crate
/// that the user wants to test.
pub fn get_ktest_crate_whitelist() -> Option<&'static [&'static str]> {
    // SAFETY: The two extern statics in the base crate are generated by OSDK.
    unsafe { KTEST_CRATE_WHITELIST }
}
