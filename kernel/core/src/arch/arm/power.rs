// SPDX-License-Identifier: MPL-2.0

use ostd::power::ExitCode;

pub(crate) fn restart_policy(code: ExitCode) {
    ostd::arch::power::try_restart(code);
    crate::power::invoke_restart_providers(code);
}

pub(crate) fn poweroff_policy(code: ExitCode) {
    ostd::arch::power::try_poweroff(code);
    crate::power::invoke_poweroff_providers(code);
}
