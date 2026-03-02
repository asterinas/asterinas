// SPDX-License-Identifier: MPL-2.0

#![doc = include_str!("../README.md")]
#![no_std]
#![deny(unsafe_code)]

pub use zerocopy::{FromBytes, FromZeros, Immutable, IntoBytes, KnownLayout};

pub mod array_helper;

/// A trait for plain old data (POD).
///
/// A POD type `T: Pod` can be safely converted to and from an arbitrary byte
/// sequence of length [`size_of::<T>()`].
/// For example, primitive types such as `u8` and `i16` are POD types.
///
/// See the crate-level documentation for design notes and usage guidance.
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
        <Self as FromBytes>::read_from_prefix(bytes).unwrap().0
    }
}

impl<T: FromBytes + IntoBytes + KnownLayout + Immutable + Copy> Pod for T {}

#[cfg(feature = "macros")]
pub use ostd_pod_macros::{derive, pod_union};
#[cfg(feature = "macros")]
pub use padding_struct::padding_struct;
