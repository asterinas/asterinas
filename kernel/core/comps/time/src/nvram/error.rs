// SPDX-License-Identifier: MPL-2.0

/// An error accessing the platform NVRAM.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    /// No supported NVRAM backend is available.
    Unavailable,
    /// The stored checksum does not match the NVRAM contents.
    InvalidChecksum,
}
