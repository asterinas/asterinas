// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{prelude::*, process::posix_thread::AsPosixThread};

pub fn sys_sync(_ctx: &Context) -> Result<SyscallReturn> {
    let current_thread = current_thread!();
    let posix_thread = current_thread.as_posix_thread().unwrap();
    let namespaces = posix_thread.namespaces().lock();
    let mnt_ns = namespaces.mnt_ns().inner();
    mnt_ns.sync(crate::fs::rootfs::root_mount().clone())?;
    Ok(SyscallReturn::Return(0))
}
