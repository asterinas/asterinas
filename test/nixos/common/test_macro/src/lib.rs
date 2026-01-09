// SPDX-License-Identifier: MPL-2.0

use proc_macro::TokenStream;
use quote::quote;
use syn::{ItemFn, parse_macro_input};

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
