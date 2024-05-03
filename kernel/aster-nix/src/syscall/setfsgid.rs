// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    prelude::*,
    process::{credentials_mut, Gid},
};

pub fn sys_setfsgid(gid: i32) -> Result<SyscallReturn> {
    debug!("gid = {}", gid);

    let fsgid = if gid < 0 {
        None
    } else {
        Some(Gid::new(gid as u32))
    };

    let old_fsgid = {
        let credentials = credentials_mut();
        credentials.set_fsgid(fsgid)?
    };

    Ok(SyscallReturn::Return(old_fsgid.as_u32() as _))
}
