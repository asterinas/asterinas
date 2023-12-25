//! # The kernel mode testing framework of Asterinas.
//!
//! `ktest` stands for kernel-mode testing framework. Its goal is to provide a
//! `cargo test`-like experience for any `#![no_std]` bare metal crates.
//!
//! In Asterinas, all the tests written in the source tree of the crates will be run
//! immediately after the initialization of aster-frame. Thus you can use any
//! feature provided by the frame including the heap allocator, etc.
//!
//! By all means, ktest is an individule crate that only requires:
//!  - a custom linker script section `.ktest_array`,
//!  - and an alloc implementation.
//! to work. And the frame happens to provide both of them. Thus, any crates depending
//! on the frame can use ktest without any extra dependency.
//!
//! ## Usage
//!
//! To write a unit test for any crates, it is recommended to create a new test
//! module, e.g.:
//!
//! ```rust
//! use ktest::{ktest, if_cfg_ktest};
//! #[if_cfg_ktest]
//! mod test {
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
//! And also, any crates using the ktest framework should be linked with aster-frame
//! and import the `ktest` crate:
//!
//! ```toml
//! # Cargo.toml
//! [dependencies]
//! ktest = { path = "relative/path/to/ktest" }
//! ```
//!
//! By the way, `#[ktest]` attribute along also works, but it hinders test control
//! using cfgs since plain attribute marked test will be executed in all test runs
//! no matter what cfgs are passed to the compiler. More importantly, using `#[ktest]`
//! without cfgs occupies binary real estate since the `.ktest_array` section is not
//! explicitly stripped in normal builds.
//!
//! Rust cfg is used to control the compilation of the test module. In cooperation
//! with the `ktest` framework, the Makefile will set the `RUSTFLAGS` environment
//! variable to pass the cfgs to all rustc invocations. To run the tests, you simply
//! need to set a list of cfgs by specifying `KTEST=1` to the Makefile, e.g.:
//!
//! ```bash
//! make run KTEST=1
//! ```
//!
//! Also, you can run a subset of tests by specifying the `KTEST_WHITELIST` variable.
//! This is achieved by a whitelist filter on the test name.
//!
//! ```bash
//! make run KTEST=1 KTEST_WHITELIST=failing_assertion,aster_frame::test::expect_panic
//! ```
//!
//! `KTEST_CRATES` variable is used to specify in which crates the tests to be run.
//! This is achieved by conditionally compiling the test module using the `#[cfg]`.
//!
//! ```bash
//! make run KTEST=1 KTEST_CRATES=aster-frame
//! ``
//!
//! We support the `#[should_panic]` attribute just in the same way as the standard
//! library do, but the implementation is quite slow currently. Use it with cautious.
//!
//! Doctest is not taken into consideration yet, and the interface is subject to
//! change.
//!

#![cfg_attr(not(test), no_std)]
#![feature(panic_info_message)]

pub mod path;
pub mod runner;
pub mod tree;

extern crate alloc;
use alloc::{boxed::Box, string::String};
use core::result::Result;

pub use ktest_proc_macro::{if_cfg_ktest, ktest};

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

#[derive(Clone)]
pub enum KtestError {
    Panic(Box<PanicInfo>),
    ShouldPanicButNoPanic,
    ExpectedPanicNotMatch(&'static str, Box<PanicInfo>),
    Unknown,
}

#[derive(Clone, PartialEq, Debug)]
pub struct KtestItemInfo {
    pub module_path: &'static str,
    pub fn_name: &'static str,
    pub package: &'static str,
    pub source: &'static str,
    pub line: usize,
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
        // Safety: __ktest_array is a static section consisting of KtestItem.
        unsafe { core::slice::from_raw_parts(__ktest_array as *const KtestItem, l) }
    }};
}

pub struct KtestIter {
    index: usize,
}

impl KtestIter {
    fn new() -> Self {
        Self { index: 0 }
    }
}

impl core::iter::Iterator for KtestIter {
    type Item = KtestItem;

    fn next(&mut self) -> Option<Self::Item> {
        let Some(ktest_item) = ktest_array!().get(self.index) else {
            return None;
        };
        self.index += 1;
        Some(ktest_item.clone())
    }
}
