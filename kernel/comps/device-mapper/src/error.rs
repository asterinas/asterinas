// SPDX-License-Identifier: MPL-2.0

use alloc::string::String;

/// Errors returned by the device-mapper component.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DmError {
    DeviceExists,
    DeviceNotFound,
    InvalidArgument,
    InvalidTable,
    NoDeviceId,
    UnsupportedTarget,
}

impl DmError {
    pub fn context(self, message: impl Into<String>) -> DmErrorWithContext {
        DmErrorWithContext {
            kind: self,
            message: message.into(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DmErrorWithContext {
    pub kind: DmError,
    pub message: String,
}
