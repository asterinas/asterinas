// SPDX-License-Identifier: MPL-2.0

//! A procedural macro to register NixOS test cases.
//!
//! This crate should work together with `nixos_test_framework` crate. The
//! registered test cases will be collected and run by the test framework.

#![deny(unsafe_code)]

use proc_macro::TokenStream;
use quote::quote;
use syn::{ItemFn, parse_macro_input};

/// Registers a function as a NixOS test case.
///
/// # Examples
/// ```rust,no_run
/// use nixos_test_framework::{Error, Session};
/// use nixos_test_macro::nixos_test;
///
/// #[nixos_test]
/// fn my_test(nixos_shell: &mut Session) -> Result<(), Error> {
///     nixos_shell.run_cmd("true")?;
///     Ok(())
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
