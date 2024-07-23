// SPDX-License-Identifier: MPL-2.0

use ostd::cpu::*;

use crate::prelude::*;

pub fn log_trap_info(trap_info: &CpuExceptionInfo) {
    trace!("[Trap][err = {:?}]", trap_info.code)
}
