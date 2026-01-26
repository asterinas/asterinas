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
//! ```
//! use ostd_pod_macros::derive;
//!
//! #[derive(Pod, Clone, Copy)]
//! struct MyStruct {
//!     // fields...
//!     a: u32
//! }
//! ```
//!
//! It expands to:
//!
//! ```
//! #[derive(::zerocopy::FromBytes, ::zerocopy::IntoBytes,
//!          ::zerocopy::Immutable, ::zerocopy::KnownLayout, Clone, Copy)]
//! struct MyStruct {
//!     // fields...
//!     a: i32
//! }
//! ```
//!
//! Note: unlike typical derive procedural macros,
//! `derive` in this crate is actually an *attribute* macro.
//! Regular derive procedural macros cannot be substituted for other derive macros.
//! This macro works by shadowing [`macro@::core::prelude::v1::derive`],
//! effectively overriding the built-in `derive` for the items where it is in scope.
//!
//! ## Example
//!
//! ```
//! use ostd_pod_macros::derive;
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
//! ## Examples
//!
//! ```ignore
//! use ostd_pod_macros::{derive, pod_union};
//! use ostd_pod::{FromZeros, IntoBytes, FromBytes};
//!
//! #[repr(C)]
//! #[pod_union]
//! #[derive(Copy, Clone)]
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

mod pod_derive;
mod pod_union;

#[cfg(test)]
extern crate self as ostd_pod_macros;

/// An attribute macro that replaces `#[derive(Pod)]` with the corresponding zerocopy traits.
///
/// # Examples
///
/// ```
/// use ostd_pod_macros::derive;
///
/// #[derive(Pod, Clone, Copy)]
/// struct S1 {
///     a: u32
/// }
/// ```
///
/// Will be transformed to:
/// ```
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
