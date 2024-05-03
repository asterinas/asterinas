// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    prelude::*,
    process::{credentials_mut, Uid},
};

pub fn sys_setuid(uid: i32) -> Result<SyscallReturn> {
    debug!("uid = {}", uid);

    if uid < 0 {
        return_errno_with_message!(Errno::EINVAL, "uid cannot be negative");
    }

    let uid = Uid::new(uid as u32);

    let credentials = credentials_mut();
    credentials.set_uid(uid);

    Ok(SyscallReturn::Return(0))
}
