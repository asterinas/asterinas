// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::prelude::*;

pub(super) fn sys_setsid(_ctx: &Context) -> Result<SyscallReturn> {
    let sid = current!().to_new_session()?;

    Ok(SyscallReturn::Return(sid as _))
}
