//! # The kernel mode testing framework of Jinux.
//!
//! `ktest` stands for kernel-mode testing framework. Its goal is to provide a
//! `cargo test`-like experience for any crates that depends on jinux-frame.
//!
//! All the tests written in the source tree of the crates will be run using the
//! `do_ktests!()` macro immediately after the initialization of jinux-frame.
//! Thus you can use any feature provided by the frame including the heap
//! allocator, etc.
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
//! }
//! ```
//!
//! And also, any crates using the ktest framework should be linked with jinux-frame
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
//! variable to pass the cfgs to all rustc invocations. To run the tests, you need
//! to pass a list of cfgs to the Makefile, e.g.:
//!
//! ```bash
//! make run KTEST=jinux-frame,jinux-std,align_ext,tdx-guest
//! ```
//!
//! It is flexible to specify the cfgs for running the tests. The cfg value is not
//! limited to crate names, enabling your imagination to configure running any subsets
//! of tests in any crates. And to ease development, `#[if_cfg_ktest]` is expanded to
//! a default conditional compilation setting:
//! `#[cfg(all(ktest, any(ktest = "all", ktest = #crate_name)))]`
//!
//! Currently we do not support `#[should_panic]` attribute, and this feature will
//! be added in the future.
//!
//! Doctest is not taken into consideration yet, and the interface is subject to
//! change.
//!
//! ## How it works
//!
//! The `ktest` framework is implemented using the procedural macro feature of Rust.
//! The `ktest` attribute macro will generate a static fn pointer variable linked in
//! the `.ktest_array` section. The `do_ktests!()` macro will iterate over all the
//! static variables in the section and run the tests.
//!

#![feature(proc_macro_span)]

extern crate proc_macro2;

use proc_macro::TokenStream;
use quote::quote;
use rand::{distributions::Alphanumeric, Rng};
use syn::{parse_macro_input, Ident, ItemFn, ItemMod};

/// The conditional compilation attribute macro to control the compilation of test
/// modules.
#[proc_macro_attribute]
pub fn if_cfg_ktest(_attr: TokenStream, item: TokenStream) -> TokenStream {
    // Assuming that the item is a module declearation, otherwise panics.
    let input = parse_macro_input!(item as ItemMod);

    let crate_name = std::env::var("CARGO_PKG_NAME").unwrap();

    let output = quote! {
        #[cfg(all(ktest, any(ktest = "all", ktest = #crate_name)))]
        #input
    };

    TokenStream::from(output)
}

/// The test attribute macro to mark a test function.
#[proc_macro_attribute]
pub fn ktest(_attr: TokenStream, item: TokenStream) -> TokenStream {
    // Assuming that the item has type `fn() -> ()`, otherwise panics.
    let input = parse_macro_input!(item as ItemFn);
    assert!(
        input.sig.inputs.is_empty(),
        "ktest function should have no arguments"
    );
    assert!(
        matches!(input.sig.output, syn::ReturnType::Default),
        "ktest function should return `()`"
    );

    // Generate a random identifier to avoid name conflicts.
    let fn_id: String = rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(8)
        .map(char::from)
        .collect();

    let fn_name = &input.sig.ident;
    let fn_ktest_item_name = Ident::new(
        &format!("{}_ktest_item_{}", &input.sig.ident, &fn_id),
        proc_macro2::Span::call_site(),
    );

    // Since Rust does not support unamed structures, we have to generate a
    // unique name for each test item structure.
    let ktest_item_struct = Ident::new(
        &format!("KtestItem{}", &fn_id),
        proc_macro2::Span::call_site(),
    );

    let span = proc_macro::Span::call_site();
    let source = span.source_file();
    let crate_name = std::env::var("CARGO_PKG_NAME").unwrap();
    let hint_str = format!(
        "[{}] {}: {}()",
        crate_name,
        source.path().to_str().unwrap(),
        fn_name
    );

    let register = quote! {
        struct #ktest_item_struct {
            fn_: fn() -> (),
            hint: &'static str,
        }
        #[cfg(ktest)]
        #[used]
        #[link_section = ".ktest_array"]
        static #fn_ktest_item_name: #ktest_item_struct = #ktest_item_struct {
            fn_: #fn_name,
            hint: #hint_str,
        };
    };

    let output = quote! {
        #input

        #register
    };

    TokenStream::from(output)
}

/// The procedural macro to run all the tests.
#[proc_macro]
pub fn do_ktests(_item: TokenStream) -> TokenStream {
    let body = quote! {
        struct KtestItem {
            fn_: fn() -> (),
            hint: &'static str,
        };
        extern "C" {
            fn __ktest_array();
            fn __ktest_array_end();
        }
        let item_size = core::mem::size_of::<KtestItem>() as u64;
        let l = (__ktest_array_end as u64 - __ktest_array as u64) / item_size;
        crate::println!("Running {} tests", l);
        for i in 0..l {
            unsafe {
                let address = (__ktest_array as u64 + item_size * i) as *const u64;
                let item = address as *const KtestItem;
                crate::print!("{} ...", (*item).hint);
                ((*item).fn_)();
            }
            crate::println!(" Ok!");
        }
        crate::exit_qemu(crate::QemuExitCode::Success);
    };

    TokenStream::from(body)
}
