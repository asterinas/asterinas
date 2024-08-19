// SPDX-License-Identifier: MPL-2.0

use crate::prelude::*;

/// Shutdown types
/// From <https://elixir.bootlin.com/linux/v6.0.9/source/include/linux/net.h>
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, TryFromInt)]
#[allow(non_camel_case_types)]
pub enum SockShutdownCmd {
    /// Shutdown receptions
    SHUT_RD = 0,
    /// Shutdown transmissions
    SHUT_WR = 1,
    /// Shutdown receptions and transmissions
    SHUT_RDWR = 2,
}

impl SockShutdownCmd {
    pub fn shut_read(&self) -> bool {
        *self == Self::SHUT_RD || *self == Self::SHUT_RDWR
    }

    pub fn shut_write(&self) -> bool {
        *self == Self::SHUT_WR || *self == Self::SHUT_RDWR
    }
}
