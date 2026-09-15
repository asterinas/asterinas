// SPDX-License-Identifier: MPL-2.0

//! Power management.

use crate::power::ExitCode;

/// Attempts to power off the system using an architecture-specific mechanism.
///
/// On LoongArch, this function currently does nothing and returns.
pub fn try_poweroff(_code: ExitCode) {
    // TODO: Add an OSTD-level poweroff mechanism for LoongArch.
}

/// Attempts to restart the system using an architecture-specific mechanism.
///
/// On LoongArch, this function currently does nothing and returns.
pub fn try_restart(_code: ExitCode) {
    // TODO: Add an OSTD-level restart mechanism for LoongArch.
}
