// SPDX-License-Identifier: MPL-2.0

//! Access information passed to a device before admitting an open request.

use crate::fs::file::{AccessMode, CreationFlags};

/// The requested access mode and exclusive-open flag for a device.
#[derive(Clone, Copy, Debug)]
pub struct DeviceOpenContext {
    access_mode: AccessMode,
    exclusive: bool,
}

impl DeviceOpenContext {
    pub(crate) fn new(access_mode: AccessMode, creation_flags: CreationFlags) -> Self {
        Self {
            access_mode,
            exclusive: creation_flags.contains(CreationFlags::O_EXCL),
        }
    }

    /// Returns whether the open request permits reading.
    pub fn is_readable(&self) -> bool {
        self.access_mode.is_readable()
    }

    /// Returns whether the open request permits writing.
    pub fn is_writable(&self) -> bool {
        self.access_mode.is_writable()
    }

    /// Returns whether the caller requested `O_EXCL`.
    pub fn is_exclusive(&self) -> bool {
        self.exclusive
    }
}
