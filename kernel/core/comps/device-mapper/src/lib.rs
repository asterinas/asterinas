// SPDX-License-Identifier: MPL-2.0

//! Device Mapper identities and device-number ownership.
//!
//! The Linux ioctl ABI lives in the kernel device layer. This component owns
//! tableless devices, their indexes, and control-reference admission and drain.
//! Mapping tables, runtime block-device publication, and I/O are not supported yet.

#![no_std]
#![deny(unsafe_code)]

extern crate alloc;

mod mapped_device;
mod registry;
#[cfg(ktest)]
mod tests;

pub use mapped_device::{DmControlRef, DmDevice};
pub use registry::DmManager;

/// Reasons why a core Device Mapper operation failed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DmError {
    /// The name already exists.
    NameExists,
    /// The UUID already exists.
    UuidExists,
    /// The specified device was not found.
    DeviceNotFound,
    /// The specified minor number is busy.
    MinorBusy,
    /// Minor numbers are exhausted or out of range.
    MinorExhausted,
    /// Block device major numbers are exhausted.
    MajorExhausted,
}
