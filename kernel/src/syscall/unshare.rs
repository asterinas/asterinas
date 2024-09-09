// SPDX-License-Identifier: MPL-2.0
use super::SyscallReturn;
use crate::{
    prelude::*,
    process::{unshare, CloneFlags},
};

pub fn sys_unshare(unshare_flags: u64, _ctx: &Context) -> Result<SyscallReturn> {
    let unshare_flags = CloneFlags::from(unshare_flags);
    debug!("flags = {:?}", unshare_flags);
    unshare(unshare_flags)?;
    Ok(SyscallReturn::Return(0))
}
