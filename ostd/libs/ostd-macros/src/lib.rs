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

        #[expect(unused)]
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

/// A macro attribute for the global frame allocator.
///
/// The attributed static variable will be used to provide frame allocation
/// for the kernel.
///
/// # Example
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
        #[no_mangle]
        static __GLOBAL_FRAME_ALLOCATOR_REF: &'static dyn ostd::mm::frame::GlobalFrameAllocator = &#static_name;
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
/// # Example
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
        #[no_mangle]
        static __GLOBAL_HEAP_ALLOCATOR_REF: &'static dyn ostd::mm::heap::GlobalHeapAllocator = &#static_name;
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
    // `const extern "Rust" fn __GLOBAL_HEAP_SLOT_INFO_FROM_LAYOUT(layout: Layout) -> Option<SlotInfo> { ... }`.
    // Reject if the input is not a `const fn`.
    let item = parse_macro_input!(item as syn::ItemFn);
    assert!(
        item.sig.constness.is_some(),
        "the annotated function must be `const`"
    );

    quote!(
        #[export_name = "__GLOBAL_HEAP_SLOT_INFO_FROM_LAYOUT"]
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
        #[no_mangle]
        extern "Rust" fn __ostd_panic_handler(info: &core::panic::PanicInfo) -> ! {
            #handler_fn_name(info);
        }

        #[expect(unused)]
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
    let span = proc_macro2::Span::call_site();
    let line = span.start().line;
    let col = span.start().column;

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
                    source: file!(),
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
                    source: file!(),
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
