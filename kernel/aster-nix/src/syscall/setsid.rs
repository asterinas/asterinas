// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::prelude::*;

pub fn sys_setsid(_ctx: &Context) -> Result<SyscallReturn> {
    let current = current!();
    let session = current.to_new_session()?;

    Ok(SyscallReturn::Return(session.sid() as _))
}
