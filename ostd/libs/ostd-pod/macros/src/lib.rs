// SPDX-License-Identifier: MPL-2.0

//! Procedural macros for the `ostd-pod` crate.
//!
//! This crate provides procedural macros to simplify working with Plain Old Data (POD)
//! types. It exports two main macros:
//!
//! - [`derive`]: An attribute macro that expands `#[derive(Pod)]` into the underlying `zerocopy` traits
//! - [`pod_union`]: An attribute macro that makes unions safe to use as POD types
//!
//! # The `derive` Macro
//!
//! The `#[derive(Pod)]` macro is a convenience wrapper that automatically derives the
//! required [`zerocopy`] traits for POD types. When you write:
//!
//! ```ignore
//! #[derive(Pod, Clone, Copy)]
//! struct MyStruct {
//!     // fields...
//! }
//! ```
//!
//! It expands to:
//!
//! ```ignore
//! #[derive(::zerocopy::FromBytes, ::zerocopy::IntoBytes,
//!          ::zerocopy::Immutable, ::zerocopy::KnownLayout, Clone, Copy)]
//! struct MyStruct {
//!     // fields...
//! }
//! ```
//!
//! ## Example
//!
//! ```ignore
//! #[macro_use]
//! extern crate ostd_pod;
//! use ostd_pod::*;
//!
//! #[repr(C)]
//! #[derive(Pod, Clone, Copy)]
//! struct Point {
//!     x: i32,
//!     y: i32,
//! }
//! ```
//!
//! # The `pod_union` Macro
//!
//! The `#[pod_union]` attribute macro enables safe usage of unions as POD types.
//! It automatically:
//!
//! - Derives the necessary [`zerocopy`] traits
//! - Generates safe accessor methods for each union field
//! - Enforces `#[repr(C)]` layout
//! - Ensures all fields are POD types
//!
//! ## Generated Accessors
//!
//! For each field `foo` in the union, the macro generates:
//!
//! - `fn foo(&self) -> &FieldType`: Returns a reference to the field
//! - `fn foo_mut(&mut self) -> &mut FieldType`: Returns a mutable reference to the field
//!
//! These accessors use `zerocopy`'s safe byte conversion methods, avoiding unsafe code.
//!
//! ## Example
//!
//! ```ignore
//! #[macro_use]
//! extern crate ostd_pod;
//! use ostd_pod::*;
//!
//! #[pod_union]
//! #[derive(Copy, Clone)]
//! #[repr(C)]
//! union Data {
//!     value: u64,
//!     bytes: [u8; 8],
//! }
//!
//! let mut data = Data::new_zeroed();
//! *data.value_mut() = 0x1234567890ABCDEF;
//!
//! // Access the same memory through different fields
//! println!("Value: 0x{:x}", *data.value());
//! println!("Bytes: {:?}", data.bytes());
//! ```
//!

use proc_macro::TokenStream;
use quote::ToTokens;
use syn::{Attribute, Path, Token, punctuated::Punctuated};

mod pod_derive;
mod pod_union;

const DERIVE_IDENT: &str = "derive";
const REPR_IDENT: &str = "repr";
const REPR_C: &str = "C";

/// A derive attribute macro that replaces `Pod` with the corresponding zerocopy traits.
///
/// # Example
/// ```ignore
/// #[derive(Pod, Clone, Copy)]
/// struct S1 {
///     a: u32
/// }
/// ```
///
/// Will be transformed to:
/// ```ignore
/// #[::core::prelude::v1::derive(::zerocopy::FromBytes, ::zerocopy::IntoBytes, ::zerocopy::Immutable, ::zerocopy::KnownLayout, Clone, Copy)]
/// struct S1 {
///     a: u32
/// }
/// ```
#[proc_macro_attribute]
pub fn derive(attrs: TokenStream, input: TokenStream) -> TokenStream {
    pod_derive::expand_derive(attrs, input)
}

#[proc_macro_attribute]
pub fn pod_union(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(item as syn::DeriveInput);
    pod_union::expand_pod_union(input).into()
}

fn split_attrs(attrs: Vec<Attribute>) -> (Vec<Attribute>, Vec<Path>) {
    let mut other_attrs = Vec::new();
    let mut derive_paths = Vec::new();

    for attr in attrs {
        if attr.path().is_ident(DERIVE_IDENT) {
            let parsed: Punctuated<Path, Token![,]> = attr
                .parse_args_with(Punctuated::parse_terminated)
                .expect("failed to parse derive attribute");
            derive_paths.extend(parsed.into_iter());
        } else {
            other_attrs.push(attr);
        }
    }

    (other_attrs, derive_paths)
}

fn insert_if_absent(paths: &mut Vec<Path>, new_path: Path) {
    let new_repr = new_path.to_token_stream().to_string();
    if !paths
        .iter()
        .any(|path| path.to_token_stream().to_string() == new_repr)
    {
        paths.push(new_path);
    }
}

fn has_repr_c(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|attr| {
        if !attr.path().is_ident(REPR_IDENT) {
            return false;
        }
        let mut has_c = false;
        let _ = attr.parse_nested_meta(|meta| {
            if meta.path.is_ident(REPR_C) {
                has_c = true;
            }
            Ok(())
        });
        has_c
    })
}
