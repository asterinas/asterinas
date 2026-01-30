// SPDX-License-Identifier: MPL-2.0

//! A marker trait and derive macros for Plain Old Data (POD) types.
//!
//! This crate provides the [`Pod`] trait, which marks types that can be safely
//! converted to and from arbitrary byte sequences. It's built on top of the
//! [`zerocopy`] crate to ensure type safety.
//!
//! # What is a POD Type?
//!
//! A POD (Plain Old Data) type is a type that can be safely converted to and from
//! an arbitrary byte sequence. For example, primitive types like `u8` and `i16` are
//! POD types.
//!
//! # Examples
//!
//! ## Define a POD Struct
//!
//! ```
//! use ostd_pod::{derive, IntoBytes, Pod};
//!
//! #[repr(C)]
//! #[derive(Pod, Clone, Copy, Debug)]
//! struct Point {
//!     x: i32,
//!     y: i32,
//! }
//!
//! # fn main() {
//! let point = Point { x: 10, y: 20 };
//!
//! // Convert to bytes
//! let bytes = point.as_bytes();
//! println!("Bytes: {:?}", bytes);
//!
//! // Create from bytes
//! let point2 = Point::from_bytes(bytes);
//! println!("Point: {:?}", point2);
//! # }
//! ```
//!
//! ## Use POD Unions
//!
//! ```rust
//! use ostd_pod::{pod_union, FromZeros, IntoBytes};
//!
//! #[pod_union]
//! #[derive(Copy, Clone)]
//! #[repr(C)]
//! union Data {
//!     value: u64,
//!     bytes: [u8; 8],
//! }
//!
//! # fn main() {
//! let mut data = Data::new_zeroed();
//! *data.value_mut() = 0x1234567890ABCDEF;
//!
//! // Access the same memory through different fields
//! println!("Value: 0x{:x}", *data.value());
//! println!("Bytes: {:?}", data.bytes());
//! # }
//! ```
//!
//! ## Automatic Padding Handling
//!
//! When a struct has fields with different sizes, there may be implicit padding bytes
//! between fields. The [`macro@padding_struct`] macro automatically inserts explicit
//! padding fields so the struct can be safely used as a POD type.
//!
//! ```rust
//! use ostd_pod::{derive, padding_struct, IntoBytes};
//!
//! #[repr(C)]
//! #[padding_struct]
//! #[derive(Pod, Clone, Copy, Debug, Default)]
//! struct PackedData {
//!     a: u8,
//!     // `padding_struct` automatically inserts 3 bytes of padding here
//!     b: u32,
//!     c: u16,
//!     // `padding_struct` automatically inserts 2 bytes of padding here
//! }
//!
//! # fn main() {
//! let data = PackedData {
//!     a: 1,
//!     b: 2,
//!     c: 3,
//!     ..Default::default()
//! };
//!
//! // Can safely convert to bytes, padding bytes are explicitly handled
//! let bytes = data.as_bytes();
//! assert_eq!(bytes.len(), 12);
//! println!("Bytes: {:?}", bytes);
//! # }
//! ```
//!
//! # Implementation Details
//!
//! This crate provides two convenient attribute macros for deriving POD traits
//! and handling union types securely.
//!
//! ## The `derive` Attribute Macro
//!
//! This crate provides the `#[derive(Pod)]` attribute macro, which is equivalent to
//! `#[derive(IntoBytes, FromBytes, KnownLayout, Immutable)]`. Unlike typical derive
//! procedural macros, [`derive`] in this crate is an **attribute** macro. Regular derive
//! procedural macros cannot be substituted for other derive macros. This macro works by
//! shadowing [`::core::prelude::v1::derive`], effectively overriding the built-in `derive`
//! for the items where it is in scope.
//!
//! ## Union Support: Transformation to Struct Wrapper
//!
//! Rust's built-in unions cannot directly derive [`zerocopy::IntoBytes`] trait because unions
//! require field-by-field initialization and access. The [`macro@pod_union`] macro
//! solves this by transforming a union into a safe wrapper struct.
//!
//! ### Transformation Example
//!
//! When you write:
//!
//! ```
//! use ostd_pod::pod_union;
//!
//! #[pod_union]
//! #[repr(C)]
//! #[derive(Clone, Copy)]
//! union Data {
//!     value: u64,
//!     bytes: [u8; 8],
//! }
//! ```
//!
//! The `#[pod_union]` macro internally generates something equivalent to:
//!
//! ```
//! // Internal private union (derived with zerocopy traits)
//! use ostd_pod::{FromBytes, KnownLayout, Immutable, FromZeros, Pod, AlignedBytes, IntoBytes};
//!
//! #[repr(C)]
//! #[derive(FromBytes, KnownLayout, Immutable)]
//! union __Data__ {
//!     value: u64,
//!     bytes: [u8; 4],
//! }
//!
//! const SIZE: usize = size_of::<__Data__>();
//!
//! // Public wrapper struct that provides safe access
//! #[repr(transparent)]
//! #[derive(FromBytes, KnownLayout, Immutable)]
//! pub struct Data(AlignedBytes<__Data__, SIZE>);
//!
//! unsafe impl IntoBytes for Data {
//!     fn only_derive_is_allowed_to_implement_this_trait() {}
//! }
//!
//! impl Data {
//!     // Field accessor methods
//!     pub fn value(&self) -> &u64 { <u64 as Pod>::ref_from_bytes(&self.0.as_bytes()[..8]) }
//!     pub fn value_mut(&mut self) -> &mut u64 { <u64 as Pod>::mut_from_bytes(&mut self.0.as_mut_bytes()[..8]) }
//!     pub fn bytes(&self) -> &[u8; 4] { <[u8; 4] as Pod>::ref_from_bytes(&self.0.as_bytes()[..4]) }
//!     pub fn bytes_mut(&mut self) -> &mut [u8; 4] { <[u8; 4] as Pod>::mut_from_bytes(&mut self.0.as_mut_bytes()[..4]) }
//!
//!     // Initializer methods
//!     pub fn new_value(value: u64) -> Self {
//!        let mut slf = Self::new_zeroed();
//!        *slf.value_mut() = value;
//!        slf
//!     }
//!     pub fn new_bytes(bytes: [u8; 4]) -> Self {
//!         let mut slf = Self::new_zeroed();
//!         *slf.bytes_mut() = bytes;
//!         slf
//!     }
//! }
//! ```
//!
//! ### Key Design Points
//!
//! - **Internal Union**: The actual union is private and derives [`zerocopy`] traits directly.
//! - **Transparent Wrapper**: The public `struct` uses `#[repr(transparent)]` to maintain
//!   the same memory layout as the internal union, ensuring zero-cost abstraction.
//! - **Field Access Methods**: Safe accessor methods (`value()`, `value_mut()`) provide
//!   type-safe access to union fields without direct union syntax.
//! - **Initializer Methods**: Convenience methods like `new_value()` allow initialization
//!   through specific fields while other fields are zeroed.
//! - **Alignment Guarantee**: The [`AlignedBytes`] wrapper ensures proper memory alignment
//!   by leveraging the internal union's alignment requirements.
//!
//! [`from_bytes`]: Pod::from_bytes
//! [`ref_from_bytes`]: Pod::ref_from_bytes
//! [`mut_from_bytes`]: Pod::mut_from_bytes
//!

#![no_std]
#![deny(unsafe_code)]

pub use aligned_bytes::AlignedBytes;
pub use zerocopy::{FromBytes, FromZeros, Immutable, IntoBytes, KnownLayout};

mod aligned_bytes;

#[cfg(test)]
extern crate self as ostd_pod;

/// A trait for plain old data (POD).
///
/// A POD type `T: Pod` can be safely converted to and from an arbitrary byte
/// sequence of length [`size_of::<T>()`].
/// For example, primitive types such as `u8` and `i16` are POD types.
///
/// The soundness of `Pod` relies on the third-party crate [`zerocopy`].
/// In practice, `Pod` is just a thin wrapper around [`zerocopy`] traits,
/// so this crate's implementation is entirely safe.
///
/// `Pod` requires that a struct contains no *implicit* padding bytes.
/// Otherwise, converting a struct to bytes would be unsound,
/// because those padding bytes may be left uninitialized.
/// This is also checked when deriving [`IntoBytes`].
///
/// Manually managing padding can be tedious, so this crate also provides
/// [`macro@padding_struct`] to automatically insert explicit padding fields.
/// See the documentation of [`padding_struct`] for details.
///
/// Manually implementing `Pod` for a struct is _discouraged_.
/// Prefer using this crate's derive macro ([`macro@derive`]) instead.
///
/// Deriving the underlying [`zerocopy`] traits directly (i.e., [`FromBytes`],
/// [`IntoBytes`], [`KnownLayout`], and [`Immutable`]) is equivalent to deriving
/// [`Pod`] via [`macro@derive`].
///
/// [`size_of::<T>()`]: size_of
pub trait Pod: FromBytes + IntoBytes + KnownLayout + Immutable + Copy {
    /// Creates a new instance from the given bytes.
    ///
    /// # Panics
    ///
    /// Panics if `bytes.len() != size_of::<Self>()`.
    #[track_caller]
    fn from_bytes(bytes: &[u8]) -> Self {
        <Self as FromBytes>::read_from_bytes(bytes).unwrap()
    }

    /// Creates a new instance by copying the first `size_of::<Self>()` bytes from `bytes`.
    ///
    /// This is useful when `bytes` contains a larger buffer (e.g., a header followed by
    /// payload) and you only want to interpret the prefix as `Self`.
    ///
    /// # Panics
    ///
    /// Panics if `bytes.len() < size_of::<Self>()`.
    #[track_caller]
    fn from_first_bytes(bytes: &[u8]) -> Self {
        let mut new_self = Self::new_zeroed();
        let copy_len = new_self.as_bytes().len();
        new_self.as_mut_bytes().copy_from_slice(&bytes[..copy_len]);
        new_self
    }

    /// Interprets the given `bytes` as a `&Self`.
    ///
    /// # Panics
    ///
    /// If the length of `bytes` is not same as the size of `Self`,
    /// or if `bytes` is not appropriately aligned, this method will panic.
    /// See also [`zerocopy::FromBytes::ref_from_bytes`].
    #[track_caller]
    fn ref_from_bytes(bytes: &[u8]) -> &Self {
        <Self as FromBytes>::ref_from_bytes(bytes).unwrap()
    }

    /// Interprets the given `bytes` as a `&mut Self`.
    ///
    /// # Panics
    ///
    /// The same as [`Pod::ref_from_bytes`].
    /// See also [`zerocopy::FromBytes::mut_from_bytes`].
    #[track_caller]
    fn mut_from_bytes(bytes: &mut [u8]) -> &mut Self {
        <Self as FromBytes>::mut_from_bytes(bytes).unwrap()
    }
}

impl<T: FromBytes + IntoBytes + KnownLayout + Immutable + Copy> Pod for T {}

#[cfg(feature = "macros")]
pub use ostd_pod_macros::{derive, pod_union};
#[cfg(feature = "macros")]
pub use padding_struct::padding_struct;
