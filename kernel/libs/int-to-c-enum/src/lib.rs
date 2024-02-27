// SPDX-License-Identifier: MPL-2.0

//! This crate provides a derive macro named TryFromInt. This macro can be used to automatically implement TryFrom trait
//! for [C-like enums](https://doc.rust-lang.org/stable/rust-by-example/custom_types/enum/c_like.html).
//!
//! Currently, this macro only supports enums with [explicit discriminants](https://doc.rust-lang.org/reference/items/enumerations.html#explicit-discriminants).
//!
//! Below is a simple example. We derive macro `TryFromInt` for an enum `Color`.
//! ```rust
//! use int_to_c_enum::TryFromInt;
//! #[repr(u8)]
//! #[derive(TryFromInt, Eq, PartialEq)]
//! pub enum Color {
//!     Red = 1,
//!     Yellow = 2,
//!     Blue = 3,
//! }
//! // Then, we can use method `try_from` for `Color`.
//! let color = Color::try_from(1).unwrap();
//! assert!(color == Color::Red);
//! ```
//!
//! The `TryFromInt` macro will automatically implement trait `TryFrom<u8>` for `Color`.
//! After macro expansion, the generated code looks like as follows:
//! ```ignore
//! impl TryFrom<u8> for Color {
//!     type Error = TryFromIntError;
//!     fn try_from(value: u8) -> Result<Self, Self::Error> {
//!         match value {
//!             1 => Ok(Color::Red),
//!             2 => Ok(Color::Yellow),
//!             3 => Ok(Color::Blue),
//!             _ => Err(TryFromIntError::InvalidValue),
//!         }
//!     }
//! }
//! ```
//!

#![cfg_attr(not(test), no_std)]

/// Error type for TryFromInt derive macro
#[derive(Debug, Clone, Copy)]
pub enum TryFromIntError {
    InvalidValue,
}

#[cfg(feature = "derive")]
pub use int_to_c_enum_derive::TryFromInt;
