// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;

pub fn sys_sysinfo(
    sysinfo_addr: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    unimplemented!("sysinfo implementation in process");
    Ok(SyscallReturn::Return(-1))
}