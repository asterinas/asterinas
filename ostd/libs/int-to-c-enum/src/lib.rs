// SPDX-License-Identifier: MPL-2.0

#![doc = include_str!("../README.md")]
#![cfg_attr(not(test), no_std)]

/// Error type for TryFromInt derive macro
#[derive(Clone, Copy, Debug)]
pub enum TryFromIntError {
    InvalidValue,
}

#[cfg(feature = "derive")]
pub use int_to_c_enum_derive::TryFromInt;
