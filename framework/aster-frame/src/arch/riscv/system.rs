// SPDX-License-Identifier: MPL-2.0

use super::sbi::srst::{system_reset, ResetReason, ResetType};

pub fn exit_success() -> ! {
    system_reset(ResetType::Shutdown, ResetReason::NoReason);
    unreachable!()
}

pub fn exit_failure() -> ! {
    system_reset(ResetType::Shutdown, ResetReason::SystemFailure);
    unreachable!()
}
