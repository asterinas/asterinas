// SPDX-License-Identifier: MPL-2.0

//! Power management.

use crate::power::{inject_poweroff_handler, inject_restart_handler, ExitCode};

fn do_poweroff(code: ExitCode) {
    let _ = match code {
        ExitCode::Success => sbi_rt::system_reset(sbi_rt::Shutdown, sbi_rt::NoReason),
        ExitCode::Failure => sbi_rt::system_reset(sbi_rt::Shutdown, sbi_rt::SystemFailure),
    };
}

fn do_restart(code: ExitCode) {
    let _ = match code {
        ExitCode::Success => sbi_rt::system_reset(sbi_rt::ColdReboot, sbi_rt::NoReason),
        ExitCode::Failure => sbi_rt::system_reset(sbi_rt::ColdReboot, sbi_rt::SystemFailure),
    };
}

pub(super) fn init() {
    inject_poweroff_handler(do_poweroff);
    inject_restart_handler(do_restart);
}
