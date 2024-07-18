// SPDX-License-Identifier: MPL-2.0

#![feature(proc_macro_span)]
#![feature(proc_macro_diagnostic)]
#![allow(dead_code)]
#![deny(unsafe_code)]

mod comp;
use comp::{component_generate, ComponentInitFunction};
use proc_macro::TokenStream;
use quote::quote;
use rand::{distributions::Alphanumeric, Rng};
use syn::{parse_macro_input, Expr, Ident, ItemFn};

/// This macro is used to mark the kernel entry point.
///
/// # Example
///
/// ```norun
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
        #[no_mangle]
        pub fn __ostd_main() -> ! {
            ostd::init();
            component::init_all(ostd::parse_metadata!(), true).unwrap();
            let test_task = move || {
                component::init_all(ostd::parse_metadata!(), false).unwrap();
                #main_fn_name();
            };
            let _ = ostd::task::TaskOptions::new(test_task).data(()).spawn();
            ostd::prelude::abort();
        }

        #main_fn
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
/// ```norun
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
/// ```norun
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

/// Register a function to be called when the component system is initialized. The function should not public.
///
/// Example:
/// ```norun
/// use ostd::prelude::*;
/// #[init_comp]
/// fn init() -> Result<(), component::ComponentInitError> {
///     Ok(())
/// }
///
/// ```
///
/// It will expand to
/// ```rust
/// fn init() -> Result<(), component::ComponentInitError> {
///     Ok(())
/// }
///
/// const fn file() -> &'static str{
///     file!()
/// }
///
/// component::submit!(component::ComponentRegistry::new(&init, file()));
/// ```
/// The priority will calculate automatically
///
#[proc_macro_attribute]
pub fn init_comp(_: TokenStream, input: TokenStream) -> proc_macro::TokenStream {
    let function = parse_macro_input!(input as ComponentInitFunction);
    let function_name = &function.function_name;

    // Generate a unique identifier
    let unique_id = format!("file_{}", function_name);
    let unique_id = syn::Ident::new(&unique_id, proc_macro2::Span::call_site());

    quote! {
        #function

        const fn #unique_id() -> &'static str {
            file!()
        }

        component::submit!(component::ComponentRegistry::new(&#function_name, #unique_id()));
    }
    .into()
}

/// Automatically generate all component information required by the component system.
///
/// It mainly uses the output of the command `cargo metadata` to automatically generate information about all components, and also checks whether `Components.toml` contains all the components.
///
/// It is often used with `component::init_all`.
///
/// Example:
///
/// ```norun
/// use ostd::prelude::*;
/// component::init_all(parse_metadata!(), false);
/// ```
///
#[proc_macro]
pub fn parse_metadata(_: TokenStream) -> proc_macro::TokenStream {
    let out = component_generate();
    quote! {
        {
            extern crate alloc;
            alloc::vec![
                #(component::ComponentInfo::new #out),*
            ]
        }
    }
    .into()
}

#[proc_macro_attribute]
pub fn init_scheduler(_: TokenStream, input: TokenStream) -> proc_macro::TokenStream {
    let function = parse_macro_input!(input as ComponentInitFunction);
    let function_name = &function.function_name;
    quote! {
        #function

        const fn file() -> &'static str{
            file!()
        }

        component::submit!(component::SchedulerRegistry::new(&#function_name, file()));
    }
    .into()
}
