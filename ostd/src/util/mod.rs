// SPDX-License-Identifier: MPL-2.0

//! Utility types and methods.

mod either;
pub mod id_set;
mod macros;
pub(crate) mod ops;
pub(crate) mod range_alloc;
pub(crate) mod range_counter;

pub use either::Either;
