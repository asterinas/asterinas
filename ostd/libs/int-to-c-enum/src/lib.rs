// SPDX-License-Identifier: MPL-2.0

#![doc = include_str!("../README.md")]
#![cfg_attr(not(test), no_std)]

/// An error indicating that an integer does not match any enum discriminant.
#[derive(Clone, Copy, Debug)]
pub struct TryFromIntError;

#[cfg(feature = "derive")]
pub use int_to_c_enum_derive::TryFromInt;
