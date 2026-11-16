// SPDX-License-Identifier: MPL-2.0

//! Device Mapper manager and block major ownership.

#![no_std]
#![deny(unsafe_code)]

extern crate alloc;

mod manager;

pub use manager::DmManager;

/// Reasons why a core Device Mapper operation failed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DmError {
    /// Block device major numbers are exhausted.
    MajorExhausted,
}
