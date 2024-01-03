// SPDX-License-Identifier: MPL-2.0

#![feature(proc_macro_span)]

extern crate proc_macro2;

use proc_macro::TokenStream;
use quote::quote;
use rand::{distributions::Alphanumeric, Rng};
use syn::{parse_macro_input, Expr, Ident, ItemFn, ItemMod};

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

    let register_ktest_item = quote! {
        #[cfg(ktest)]
        #[used]
        #[link_section = ".ktest_array"]
        static #fn_ktest_item_name: ktest::KtestItem = ktest::KtestItem::new(
            #fn_name,
            (#should_panic, #expectation_tokens),
            ktest::KtestItemInfo {
                module_path: module_path!(),
                fn_name: stringify!(#fn_name),
                package: #package_name,
                source: #source,
                line: #line,
                col: #col,
            },
        );
    };

    let output = quote! {
        #input

        #register_ktest_item
    };

    TokenStream::from(output)
}
