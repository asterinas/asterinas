// SPDX-License-Identifier: MPL-2.0

//! Implementation of OneReg-related system calls for the RISC-V architecture.

/// Errors that may occur when performing OneReg-related operations.
#[derive(Clone, Copy, Debug)]
pub enum OneRegError {
    /// Convert to `ENOENT`.
    NotFoundRegister,
    /// Convert to `EINVAL`.
    InvalidArgs,
    /// Convert to `EBUSY`.
    ResourceIsBusy,
}
