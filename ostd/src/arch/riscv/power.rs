// SPDX-License-Identifier: MPL-2.0

//! Power management.

use crate::power::ExitCode;

/// Attempts to power off the system using an architecture-specific mechanism.
///
/// On RISC-V, this function attempts to power off the system through the SBI runtime. It does not
/// return on success; if the SBI request fails, it returns without further action.
pub fn try_poweroff(code: ExitCode) {
    let _ = match code {
        ExitCode::Success => sbi_rt::system_reset(sbi_rt::Shutdown, sbi_rt::NoReason),
        ExitCode::Failure => sbi_rt::system_reset(sbi_rt::Shutdown, sbi_rt::SystemFailure),
    };
}

/// Attempts to restart the system using an architecture-specific mechanism.
///
/// On RISC-V, this function attempts to restart the system through the SBI runtime. It does not
/// return on success; if the SBI request fails, it returns without further action.
pub fn try_restart(code: ExitCode) {
    let _ = match code {
        ExitCode::Success => sbi_rt::system_reset(sbi_rt::ColdReboot, sbi_rt::NoReason),
        ExitCode::Failure => sbi_rt::system_reset(sbi_rt::ColdReboot, sbi_rt::SystemFailure),
    };
}
