// SPDX-License-Identifier: MPL-2.0

//! This crate defines two attribute macros `controlled` and `uncontrolled`.
//! This two macros are attached to functions or static variables to enable crate level access control.
//! To use these two macros, a crate must at first registers a tool named `component_access_control`,
//! because controlled used tool attribute internally.
//!
//! Below is a simple usage example.
//! ```rust
//! // crate-level tool registration
//! #![feature(register_tool)]
//! #![register_tool(component_access_control)]
//!
//! #[macro_use]
//! extern crate controlled; // import this crate
//!
//! #[controlled]
//! pub static FOO: usize = 0;
//!
//! #[uncontrolled]
//! pub fn bar() {}
//! ```
use quote::quote;

#[proc_macro_attribute]
pub fn controlled(
    attr: proc_macro::TokenStream,
    item: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    let attr = attr.to_string();
    if !attr.is_empty() {
        panic!("controlled cannot accept inner tokens.")
    }
    let mut tokens: proc_macro::TokenStream = quote!(
        #[component_access_control::controlled]
    )
    .into();
    tokens.extend(item);
    tokens
}

#[proc_macro_attribute]
pub fn uncontrolled(
    attr: proc_macro::TokenStream,
    item: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    let attr = attr.to_string();
    if !attr.is_empty() {
        panic!("uncontrolled cannot accept inner tokens.")
    }
    let mut tokens: proc_macro::TokenStream = quote!(
        #[component_access_control::uncontrolled]
    )
    .into();
    tokens.extend(item);
    tokens
}
