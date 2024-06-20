// SPDX-License-Identifier: MPL-2.0

use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, ItemFn};

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
            #main_fn_name();
            ostd::prelude::abort();
        }

        #main_fn
    )
    .into()
}
