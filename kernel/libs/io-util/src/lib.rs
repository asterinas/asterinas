// SPDX-License-Identifier: MPL-2.0

//! Low-level I/O utilities.

#![no_std]

pub mod batch;

/// A low-level I/O error.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IoError {
    /// The operation is not supported by the backend.
    Unsupported,
    /// The device has no free space.
    OutOfSpace,
    /// A generic I/O failure.
    Failed,
}
