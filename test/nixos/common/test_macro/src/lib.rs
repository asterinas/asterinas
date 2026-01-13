// SPDX-License-Identifier: MPL-2.0

//! A procedural macro to register NixOS test cases.
//!
//! This crate should work together with `nixos_test_framework` crate. The
//! registered test cases will be collected and run by the test framework.

use proc_macro::TokenStream;
use quote::quote;
use syn::{ItemFn, parse_macro_input};

/// Registers a function as a NixOS test case.
///
/// # Example
/// ```rust
/// #[nixos_test]
/// fn my_test() {
///     // test code here
/// }
/// ```
#[proc_macro_attribute]
pub fn nixos_test(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as ItemFn);
    let fn_name = &input.sig.ident;
    let fn_name_str = fn_name.to_string();

    let expanded = quote! {
        #input

        ::nixos_test_framework::inventory::submit! {
            ::nixos_test_framework::TestCase {
                name: #fn_name_str,
                test_fn: #fn_name,
            }
        }
    };

    TokenStream::from(expanded)
}
