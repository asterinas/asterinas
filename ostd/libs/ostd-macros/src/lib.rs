// SPDX-License-Identifier: MPL-2.0

#![feature(proc_macro_span)]

use proc_macro::TokenStream;
use quote::quote;
use rand::{distributions::Alphanumeric, Rng};
use syn::{parse_macro_input, Expr, Ident, ItemFn};

/// A macro attribute to mark the kernel entry point.
///
/// # Example
///
/// ```ignore
/// #![no_std]
///
/// use ostd::prelude::*;
///
/// #[ostd::main]
/// pub fn main() {
///     println!("hello world");
/// }
/// ```
#[proc_macro_attribute]
pub fn main(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let main_fn = parse_macro_input!(item as ItemFn);
    let main_fn_name = &main_fn.sig.ident;

    quote!(
        #[cfg(not(ktest))]
        #[no_mangle]
        extern "Rust" fn __ostd_main() -> ! {
            let _: () = #main_fn_name();

            ostd::task::Task::yield_now();
            unreachable!("`yield_now` in the boot context should not return");
        }

        #[allow(unused)]
        #main_fn
    )
    .into()
}

/// A macro attribute for the unit test kernel entry point.
///
/// This macro is used for internal OSDK implementation. Do not use it
/// directly.
#[proc_macro_attribute]
pub fn test_main(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let main_fn = parse_macro_input!(item as ItemFn);
    let main_fn_name = &main_fn.sig.ident;

    quote!(
        #[no_mangle]
        extern "Rust" fn __ostd_main() -> ! {
            let _: () = #main_fn_name();

            ostd::task::Task::yield_now();
            unreachable!("`yield_now` in the boot context should not return");
        }

        #main_fn
    )
    .into()
}

/// A macro attribute for the panic handler.
///
/// The attributed function will be used to override OSTD's default
/// implementation of Rust's `#[panic_handler]`. The function takes a single
/// parameter of type `&core::panic::PanicInfo` and does not return.
#[proc_macro_attribute]
pub fn panic_handler(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let handler_fn = parse_macro_input!(item as ItemFn);
    let handler_fn_name = &handler_fn.sig.ident;

    quote!(
        #[cfg(not(ktest))]
        #[no_mangle]
        extern "Rust" fn __ostd_panic_handler(info: &core::panic::PanicInfo) -> ! {
            #handler_fn_name(info);
        }

        #[allow(unused)]
        #handler_fn
    )
    .into()
}

/// A macro attribute for the panic handler.
///
/// This macro is used for internal OSDK implementation. Do not use it
/// directly.
#[proc_macro_attribute]
pub fn test_panic_handler(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let handler_fn = parse_macro_input!(item as ItemFn);
    let handler_fn_name = &handler_fn.sig.ident;

    quote!(
        #[no_mangle]
        extern "Rust" fn __ostd_panic_handler(info: &core::panic::PanicInfo) -> ! {
            #handler_fn_name(info);
        }

        #handler_fn
    )
    .into()
}

/// The test attribute macro to mark a test function.
///
/// # Example
///
/// For crates other than ostd,
/// this macro can be used in the following form.
///
/// ```ignore
/// use ostd::prelude::*;
///
/// #[ktest]
/// fn test_fn() {
///     assert_eq!(1 + 1, 2);
/// }
/// ```
///
/// For ostd crate itself,
/// this macro can be used in the form
///
/// ```ignore
/// use crate::prelude::*;
///
/// #[ktest]
/// fn test_fn() {
///     assert_eq!(1 + 1, 2);
/// }
/// ```
#[proc_macro_attribute]
pub fn ktest(_attr: TokenStream, item: TokenStream) -> TokenStream {
    // Assuming that the item has type `fn() -> ()`, otherwise panics.
    let input = parse_macro_input!(item as ItemFn);
    assert!(
        input.sig.inputs.is_empty(),
        "ostd::test function should have no arguments"
    );
    assert!(
        matches!(input.sig.output, syn::ReturnType::Default),
        "ostd::test function should return `()`"
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

    let is_should_panic_attr = |attr: &&syn::Attribute| {
        attr.path()
            .segments
            .iter()
            .any(|segment| segment.ident == "should_panic")
    };
    let mut attr_iter = input.attrs.iter();
    let should_panic = attr_iter.find(is_should_panic_attr);
    let (should_panic, expectation) = match should_panic {
        Some(attr) => {
            assert!(
                !attr_iter.any(|attr: &syn::Attribute| is_should_panic_attr(&attr)),
                "multiple `should_panic` attributes"
            );
            match &attr.meta {
                syn::Meta::List(l) => {
                    let arg_err_message = "`should_panic` attribute should only have zero or one `expected` argument, with the format of `expected = \"<panic message>\"`";
                    let expected_assign =
                        syn::parse2::<syn::ExprAssign>(l.tokens.clone()).expect(arg_err_message);
                    let Expr::Lit(s) = *expected_assign.right else {
                        panic!("{}", arg_err_message);
                    };
                    let syn::Lit::Str(expectation) = s.lit else {
                        panic!("{}", arg_err_message);
                    };
                    (true, Some(expectation))
                }
                _ => (true, None),
            }
        }
        None => (false, None),
    };
    let expectation_tokens = if let Some(s) = expectation {
        quote! {
            Some(#s)
        }
    } else {
        quote! {
            None
        }
    };

    let package_name = std::env::var("CARGO_PKG_NAME").unwrap();
    let span = proc_macro::Span::call_site();
    let source = span.source_file().path();
    let source = source.to_str().unwrap();
    let line = span.line();
    let col = span.column();

    let register_ktest_item = if package_name.as_str() == "ostd" {
        quote! {
            #[cfg(ktest)]
            #[used]
            #[link_section = ".ktest_array"]
            static #fn_ktest_item_name: ostd_test::KtestItem = ostd_test::KtestItem::new(
                #fn_name,
                (#should_panic, #expectation_tokens),
                ostd_test::KtestItemInfo {
                    module_path: module_path!(),
                    fn_name: stringify!(#fn_name),
                    package: #package_name,
                    source: #source,
                    line: #line,
                    col: #col,
                },
            );
        }
    } else {
        quote! {
            #[cfg(ktest)]
            #[used]
            #[link_section = ".ktest_array"]
            static #fn_ktest_item_name: ostd::ktest::KtestItem = ostd::ktest::KtestItem::new(
                #fn_name,
                (#should_panic, #expectation_tokens),
                ostd::ktest::KtestItemInfo {
                    module_path: module_path!(),
                    fn_name: stringify!(#fn_name),
                    package: #package_name,
                    source: #source,
                    line: #line,
                    col: #col,
                },
            );
        }
    };

    let output = quote! {
        #input

        #register_ktest_item
    };

    TokenStream::from(output)
}
