// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::prelude::*;

pub fn sys_gettid() -> Result<SyscallReturn> {
    let current_thread = current_thread!();
    let tid = current_thread.tid();
    Ok(SyscallReturn::Return(tid as _))
}
