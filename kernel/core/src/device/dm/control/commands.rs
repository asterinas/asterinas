// SPDX-License-Identifier: MPL-2.0

//! Dispatch of the supported Device Mapper control commands.

use crate::prelude::*;

type CommandHandler = fn(&mut [u8]) -> Result<()>;

/// Selects only commands implemented by this control device.
pub(super) fn handler_for(_command: u8) -> Result<CommandHandler> {
    return_errno_with_message!(Errno::ENOTTY, "unsupported device-mapper ioctl command")
}
