// SPDX-License-Identifier: MPL-2.0

#![feature(proc_macro_diagnostic)]
#![feature(proc_macro_span)]

use proc_macro::{Diagnostic, Level, Span, TokenStream};
use quote::quote;
use rand::{Rng, distr::Alphanumeric};
use syn::{Expr, Ident, ItemFn, parse_macro_input};

/// A macro attribute to mark the kernel entry point.
///
/// # Examples
///
/// ```ignore
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
    let body = ostd_main_body(main_fn_name);

    quote!(
        #[cfg(not(ktest))]
        // SAFETY: The name does not collide with other symbols.
        #[unsafe(no_mangle)]
        extern "Rust" fn __ostd_main() -> ! {
            #body
        }

        #[expect(unused)]
        #main_fn
    )
    .into()
}

/// A macro attribute to mark the unit test kernel entry point.
///
/// This macro is used for internal OSDK implementation. Do not use it
/// directly.
#[doc(hidden)]
#[proc_macro_attribute]
pub fn test_main(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let main_fn = parse_macro_input!(item as ItemFn);
    let main_fn_name = &main_fn.sig.ident;
    let body = ostd_main_body(main_fn_name);

    quote!(
        // SAFETY: The name does not collide with other symbols.
        #[unsafe(no_mangle)]
        extern "Rust" fn __ostd_main() -> ! {
            #body
        }

        #main_fn
    )
    .into()
}

fn ostd_main_body(main_fn_name: &Ident) -> proc_macro2::TokenStream {
    quote! {
        let _: () = #main_fn_name();

        ::ostd::task::Task::yield_now();

        // If we reach this point, the user-provided main function did not
        // spawn any tasks, so there is nothing left to schedule.
        // Power off gracefully.
        ::core::assert!(::ostd::task::Task::current().is_none());
        ::ostd::power::poweroff(::ostd::power::ExitCode::Success);
    }
}

/// A macro attribute for the global frame allocator.
///
/// The attributed static variable will be used to provide frame allocation
/// for the kernel.
///
/// # Examples
///
/// ```ignore
/// use core::alloc::Layout;
/// use ostd::{mm::{frame::GlobalFrameAllocator, Paddr}, global_frame_allocator};
///
/// // Of course it won't work because all allocations will fail.
/// // It's just an example.
/// #[global_frame_allocator]
/// static ALLOCATOR: MyFrameAllocator = MyFrameAllocator;
///
/// struct MyFrameAllocator;
///
/// impl GlobalFrameAllocator for MyFrameAllocator {
///     fn alloc(&self, _layout: Layout) -> Option<Paddr> { None }
///     fn dealloc(&self, _paddr: Paddr, _size: usize) {}
/// }
/// ```
#[proc_macro_attribute]
pub fn global_frame_allocator(_attr: TokenStream, item: TokenStream) -> TokenStream {
    // Make a `static __GLOBAL_FRAME_ALLOCATOR_REF: &'static dyn GlobalFrameAllocator`
    // That points to the annotated static variable.

    let item = parse_macro_input!(item as syn::ItemStatic);
    let static_name = &item.ident;

    quote!(
        // SAFETY: The name does not collide with other symbols.
        #[unsafe(no_mangle)]
        static __GLOBAL_FRAME_ALLOCATOR_REF: &'static dyn ::ostd::mm::frame::GlobalFrameAllocator =
            &#static_name;

        #item
    )
    .into()
}

/// A macro attribute to register the global heap allocator.
///
/// The attributed static variable will be used to provide heap allocation
/// for the kernel.
///
/// This attribute is not to be confused with Rust's built-in
/// [`global_allocator`] attribute, which applies to a static variable
/// implementing the unsafe `GlobalAlloc` trait. In contrast, the
/// [`macro@global_heap_allocator`] attribute does not require the heap allocator to
/// implement an unsafe trait. [`macro@global_heap_allocator`] eventually relies on
/// [`global_allocator`] to customize Rust's heap allocator.
///
/// # Examples
///
/// ```ignore
/// use core::alloc::{AllocError, Layout};
/// use ostd::{mm::heap::{GlobalHeapAllocator, HeapSlot}, global_heap_allocator};
///
/// // Of course it won't work and all allocations will fail.
/// // It's just an example.
/// #[global_heap_allocator]
/// static ALLOCATOR: MyHeapAllocator = MyHeapAllocator;
///
/// struct MyHeapAllocator;
///
/// impl GlobalHeapAllocator for MyHeapAllocator {
///     fn alloc(&self, _layout: Layout) -> Result<HeapSlot, AllocError> { None }
///     fn dealloc(&self, _slot: HeapSlot) -> Result<(), AllocError> {}
/// }
/// ```
#[proc_macro_attribute]
pub fn global_heap_allocator(_attr: TokenStream, item: TokenStream) -> TokenStream {
    // Make a `static __GLOBAL_HEAP_ALLOCATOR_REF: &'static dyn GlobalHeapAllocator`
    // That points to the annotated static variable.

    let item = parse_macro_input!(item as syn::ItemStatic);
    let static_name = &item.ident;

    quote!(
        // SAFETY: The name does not collide with other symbols.
        #[unsafe(no_mangle)]
        static __GLOBAL_HEAP_ALLOCATOR_REF: &'static dyn ::ostd::mm::heap::GlobalHeapAllocator =
            &#static_name;

        #item
    )
    .into()
}

/// A macro attribute to map allocation layouts to slot sizes and types.
///
/// In OSTD, both slab slots and large slots are used to serve heap allocations.
/// Slab slots must come from slabs of fixed sizes, while large slots can be
/// allocated by frame allocation, with sizes being multiples of pages.
/// OSTD must know the user's decision on the size and type of a slot to serve
/// an allocation with a given layout.
///
/// This macro should be used to annotate a function that maps a layout to the
/// slot size and the type. The function should return `None` if the layout is
/// not supported.
///
/// The annotated function should be idempotent, meaning the result should be the
/// same for the same layout. OSDK enforces this by only allowing the function
/// to be `const`.
#[proc_macro_attribute]
pub fn global_heap_allocator_slot_map(_attr: TokenStream, item: TokenStream) -> TokenStream {
    // Rewrite the input `const fn __any_name__(layout: Layout) -> Option<SlotInfo> { ... }` to
    // `const extern "Rust" fn __global_heap_slot_info_from_layout(layout: Layout) -> Option<SlotInfo> { ... }`.

    let item = parse_macro_input!(item as syn::ItemFn);
    let fn_name = &item.sig.ident;

    // Reject if the input is not a `const fn`.
    assert!(
        item.sig.constness.is_some(),
        "the annotated function must be `const`"
    );

    quote!(
        /// SAFETY: The name does not collide with other symbols.
        #[unsafe(no_mangle)]
        const extern "Rust" fn __global_heap_slot_info_from_layout(
            layout: ::core::alloc::Layout,
        ) -> ::core::option::Option<::ostd::mm::heap::SlotInfo> {
            #fn_name(layout)
        }

        #item
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
        // SAFETY: The name does not collide with other symbols.
        #[unsafe(no_mangle)]
        extern "Rust" fn __ostd_panic_handler(info: &::core::panic::PanicInfo) -> ! {
            #handler_fn_name(info);
        }

        #[expect(unused)]
        #handler_fn
    )
    .into()
}

/// A macro attribute for the unit test panic handler.
///
/// This macro is used for internal OSDK implementation. Do not use it
/// directly.
#[doc(hidden)]
#[proc_macro_attribute]
pub fn test_panic_handler(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let handler_fn = parse_macro_input!(item as ItemFn);
    let handler_fn_name = &handler_fn.sig.ident;

    quote!(
        // SAFETY: The name does not collide with other symbols.
        #[unsafe(no_mangle)]
        extern "Rust" fn __ostd_panic_handler(info: &::core::panic::PanicInfo) -> ! {
            #handler_fn_name(info);
        }

        #handler_fn
    )
    .into()
}

/// The test attribute macro to mark a test function.
///
/// # Examples
///
/// For crates other than `ostd`,
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
/// For `ostd` crate itself,
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
pub fn ktest(attr: TokenStream, item: TokenStream) -> TokenStream {
    // Assuming that the item has type `fn() -> ()`, otherwise panics.
    let input = parse_macro_input!(item as ItemFn);
    assert!(
        input.sig.inputs.is_empty(),
        "test functions should have no arguments"
    );
    assert!(
        matches!(input.sig.output, syn::ReturnType::Default),
        "test functions should return `()`"
    );

    // Generate a random identifier to avoid name conflicts.
    let fn_id: String = rand::rng()
        .sample_iter(&Alphanumeric)
        .take(8)
        .map(char::from)
        .collect();

    let fn_name = &input.sig.ident;
    let fn_ktest_item_name = Ident::new(
        &format!("{}_ktest_item_{}", &input.sig.ident, &fn_id),
        proc_macro2::Span::call_site(),
    );

    // Emit warnings similar to `clippy::redundant_test_prefix`.
    emit_redundant_test_prefix_warnings(attr, &input.sig.ident);

    // Deal with `#[should_panic]`.
    let panic_expectation_tokens = generate_panic_expectation_tokens(&input.attrs);

    let package_name = std::env::var("CARGO_PKG_NAME").unwrap();
    let span = proc_macro2::Span::call_site();
    let line = span.start().line;
    let col = span.start().column;

    let register_ktest_item = if package_name.as_str() == "ostd" {
        quote! {
            #[cfg(ktest)]
            #[used]
            // SAFETY: This is properly handled in the linker script.
            #[unsafe(link_section = ".ktest_array")]
            static #fn_ktest_item_name: ::ostd_test::KtestItem = ::ostd_test::KtestItem::new(
                #fn_name,
                #panic_expectation_tokens,
                ::ostd_test::KtestItemInfo {
                    module_path: ::core::module_path!(),
                    fn_name: ::core::stringify!(#fn_name),
                    package: #package_name,
                    source: ::core::file!(),
                    line: #line,
                    col: #col,
                },
            );
        }
    } else {
        quote! {
            #[cfg(ktest)]
            #[used]
            // SAFETY: This is properly handled in the linker script.
            #[unsafe(link_section = ".ktest_array")]
            static #fn_ktest_item_name: ::ostd::ktest::KtestItem = ::ostd::ktest::KtestItem::new(
                #fn_name,
                #panic_expectation_tokens,
                ::ostd::ktest::KtestItemInfo {
                    module_path: ::core::module_path!(),
                    fn_name: ::core::stringify!(#fn_name),
                    package: #package_name,
                    source: ::core::file!(),
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

fn emit_redundant_test_prefix_warnings(attr: TokenStream, ident: &Ident) {
    let should_have_test_prefix = if !attr.is_empty() {
        // This is an equivalent of `#[expect(clippy::redundant_test_prefix)]`.
        assert_eq!(
            attr.to_string(),
            "expect_redundant_test_prefix",
            "unknown arguments"
        );
        true
    } else {
        false
    };

    fn should_deny_warnings(rustflags: &str) -> bool {
        let options = rustflags.split_whitespace().collect::<Vec<_>>();

        if options.is_empty() {
            return false;
        }

        if options.contains(&"-Dwarnings") {
            return true;
        }

        for i in 0..options.len() - 1 {
            if (options[i] == "--deny" || options[i] == "-D") && options[i + 1] == "warnings" {
                return true;
            }
        }

        false
    }

    // FIXME: `-Dwarnings` cannot automatically convert warnings generated from procedural macros
    // into errors. So we do this manually. Remove this workaround when the upstream changes
    // resolve the issue (see <https://github.com/rust-lang/rust/pull/135432>).
    let lint_level = if let Ok(rustflags) = std::env::var("RUSTFLAGS")
        && should_deny_warnings(rustflags.as_str())
    {
        Level::Error
    } else {
        Level::Warning
    };

    let test_prefix_stripped = ident
        .to_string()
        .strip_prefix("test_")
        .map(ToOwned::to_owned);

    if !should_have_test_prefix && let Some(name_to_suggest) = test_prefix_stripped {
        Diagnostic::spanned(
            ident.span().unwrap(),
            lint_level,
            "redundant `test_` prefix in test function name",
        )
        .span_help(
            ident.span().unwrap(),
            format!(
                "consider removing the `test_` prefix: `{}`",
                name_to_suggest
            ),
        )
        .span_help(
            Span::call_site(),
            "consider allowing the lint: `#[ktest(expect_redundant_test_prefix)]`",
        )
        .help(
            "for further information visit \
             https://rust-lang.github.io/rust-clippy/master/index.html#redundant_test_prefix",
        )
        .emit()
    } else if should_have_test_prefix && test_prefix_stripped.is_none() {
        Diagnostic::spanned(
            ident.span().unwrap(),
            lint_level,
            "no redundant `test_` prefix in test function name",
        )
        .span_help(
            Span::call_site(),
            "consider removing the expectation: `#[ktest]`",
        )
        .emit();
    };
}

fn generate_panic_expectation_tokens(attrs: &[syn::Attribute]) -> proc_macro2::TokenStream {
    fn is_should_panic_attr(attr: &syn::Attribute) -> bool {
        attr.path()
            .segments
            .iter()
            .any(|segment| segment.ident == "should_panic")
    }

    let mut attr_iter = attrs.iter();
    let Some(should_panic_attr) = attr_iter.find(|&attr| is_should_panic_attr(attr)) else {
        let tokens = quote! { (false, None) };
        return tokens;
    };
    assert!(
        !attr_iter.any(is_should_panic_attr),
        "multiple `should_panic` attributes"
    );

    match &should_panic_attr.meta {
        syn::Meta::List(list) => {
            if let Ok(expected_assign) = syn::parse2::<syn::ExprAssign>(list.tokens.clone())
                && let Expr::Lit(lit) = *expected_assign.right
                && let syn::Lit::Str(expectation) = lit.lit
            {
                let tokens = quote! { (true, Some(#expectation)) };
                return tokens;
            }
        }
        syn::Meta::Path(_) => {
            let tokens = quote! { (true, None) };
            return tokens;
        }
        _ => (),
    }

    panic!(
        "`should_panic` attributes should only have zero or one `expected` argument, \
         with the format of `expected = \"<panic message>\"`"
    );
}
