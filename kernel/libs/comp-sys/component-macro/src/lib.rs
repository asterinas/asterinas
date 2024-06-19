// SPDX-License-Identifier: MPL-2.0

//！This crate defines the component system related macros.

#![feature(proc_macro_diagnostic)]
#![allow(dead_code)]
#![deny(unsafe_code)]

mod init_comp;
mod priority;

use init_comp::ComponentInitFunction;
use proc_macro::TokenStream;
use quote::quote;
use syn::parse_macro_input;

pub(crate) const COMPONENT_FILE_NAME: &str = "Components.toml";

/// Register a function to be called when the component system is initialized. The function should not public.
///
/// Example:
/// ```rust
/// #[init_component]
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
/// component::submit!(component::ComponentRegistry::new(&init,file()));
/// ```
/// The priority will calculate automatically
///
#[proc_macro_attribute]
pub fn init_component(_: TokenStream, input: TokenStream) -> proc_macro::TokenStream {
    let function = parse_macro_input!(input as ComponentInitFunction);
    let function_name = &function.function_name;
    quote! {
        #function

        const fn file() -> &'static str{
            file!()
        }

        component::submit!(component::ComponentRegistry::new(&#function_name,file()));
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
/// ```rust
///     component::init_all(component::parse_metadata!());
/// ```
///
#[proc_macro]
pub fn parse_metadata(_: TokenStream) -> proc_macro::TokenStream {
    let out = priority::component_generate();
    let path = priority::get_component_toml_path();
    quote! {
        {
            include_str!(#path);
            extern crate alloc;
            alloc::vec![
                #(component::ComponentInfo::new #out),*
            ]
        }
    }
    .into()
}
