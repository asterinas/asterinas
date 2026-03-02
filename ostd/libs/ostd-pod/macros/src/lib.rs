// SPDX-License-Identifier: MPL-2.0

#![doc = include_str!("../README.md")]

use proc_macro::TokenStream;

mod pod_derive;
mod pod_union;

/// An attribute macro that replaces `#[derive(Pod)]` with the corresponding zerocopy traits.
#[proc_macro_attribute]
pub fn derive(attrs: TokenStream, input: TokenStream) -> TokenStream {
    pod_derive::expand_derive(attrs, input)
}

/// An attribute macro that enables safe usage of unions as POD types.
///
/// Rust's built-in unions cannot directly derive `zerocopy::IntoBytes` because unions require
/// field-by-field initialization and access. The `#[pod_union]` macro solves this by
/// transforming a union into a safe wrapper struct.
///
/// # Implementation details
///
/// When you write:
///
/// ```rust
/// use ostd_pod_macros::pod_union;
///
/// #[repr(C)]
/// #[pod_union]
/// #[derive(Clone, Copy)]
/// pub union Data {
///     value: u64,
///     bytes: [u8; 4],
/// }
/// ```
///
/// The `#[pod_union]` macro internally generates something equivalent to:
///
/// ```rust
/// use ostd_pod::array_helper::{ArrayFactory, ArrayManufacture, U64Array};
/// use ostd_pod::{FromBytes, FromZeros, Immutable, IntoBytes, KnownLayout, Pod};
///
/// // Internal private union
/// #[repr(C)]
/// #[derive(FromBytes, KnownLayout, Immutable)]
/// union __Data__ {
///     value: u64,
///     bytes: [u8; 4],
/// }
///
/// // Public wrapper struct that provides safe access
/// #[repr(transparent)]
/// #[derive(FromBytes, KnownLayout, Immutable, IntoBytes)]
/// pub struct Data(<ArrayFactory<
///        { align_of::<__Data__>() },
///        { size_of::<__Data__>() / (align_of::<__Data__>()) },
///    > as ArrayManufacture>::Array);
///
/// impl Data {
///     // Field accessor methods
///     pub fn value(&self) -> &u64 {
///         u64::ref_from_bytes(&self.0.as_bytes()[..8]).unwrap()
///     }
///     pub fn value_mut(&mut self) -> &mut u64 {
///         u64::mut_from_bytes(&mut self.0.as_mut_bytes()[..8]).unwrap()
///     }
///     pub fn bytes(&self) -> &[u8; 4] {
///         <[u8; 4]>::ref_from_bytes(&self.0.as_bytes()[..4]).unwrap()
///     }
///     pub fn bytes_mut(&mut self) -> &mut [u8; 4] {
///         <[u8; 4]>::mut_from_bytes(&mut self.0.as_mut_bytes()[..4]).unwrap()
///     }
///
///     // Initializer methods
///     pub fn new_value(value: u64) -> Self {
///         let mut slf = Self::new_zeroed();
///         *slf.value_mut() = value;
///         slf
///     }
///     pub fn new_bytes(bytes: [u8; 4]) -> Self {
///         let mut slf = Self::new_zeroed();
///         *slf.bytes_mut() = bytes;
///         slf
///     }
/// }
/// ```
#[proc_macro_attribute]
pub fn pod_union(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(item as syn::DeriveInput);
    pod_union::expand_pod_union(input).into()
}
