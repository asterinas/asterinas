// SPDX-License-Identifier: MPL-2.0

crate::register_restart_handler!(
    ostd::arch::power::try_restart,
    crate::power::Priority::DEFAULT
);
crate::register_poweroff_handler!(
    ostd::arch::power::try_poweroff,
    crate::power::Priority::DEFAULT
);
