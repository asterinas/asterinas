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
//! ```ignore
//! #[macro_use]
//! extern crate ostd_pod;
//! use ostd_pod::*;
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

#![no_std]
#![deny(unsafe_code)]

pub use zerocopy::{FromBytes, FromZeros, Immutable, IntoBytes, KnownLayout};

/// A trait for plain old data (POD).
///
/// A POD type `T: Pod` can be safely converted to and from an arbitrary byte
/// sequence of length [`core::mem::size_of::<T>()`].
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
/// [`core::mem::size_of::<T>()`]: core::mem::size_of
pub trait Pod: FromBytes + IntoBytes + KnownLayout + Immutable + Copy {
    /// Creates a new instance from the given bytes.
    fn from_bytes(bytes: &[u8]) -> Self {
        // FIXME: Should we check if `bytes.len() == core::mem::size_of::<Self>()`?
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
    fn ref_from_bytes(bytes: &[u8]) -> &Self {
        <Self as FromBytes>::ref_from_bytes(bytes).unwrap()
    }

    /// Interprets the given `source` as a `&mut Self`.
    ///
    /// # Panics
    ///
    /// The same as [`Pod::ref_from_bytes`].
    /// See also [`zerocopy::FromBytes::mut_from_bytes`].
    fn mut_from_bytes(bytes: &mut [u8]) -> &mut Self {
        <Self as FromBytes>::mut_from_bytes(bytes).unwrap()
    }
}

impl<T: FromBytes + IntoBytes + KnownLayout + Immutable + Copy> Pod for T {}

#[cfg(feature = "macros")]
pub use ostd_pod_macros::*;
#[cfg(feature = "macros")]
pub use padding_struct::padding_struct;
