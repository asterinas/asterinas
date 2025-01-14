// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::prelude::*;

pub fn sys_show_red(_ctx: &Context) -> Result<SyscallReturn> {
    println!("Red");
    Ok(SyscallReturn::NoReturn)
}