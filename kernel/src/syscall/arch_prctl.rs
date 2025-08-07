// SPDX-License-Identifier: MPL-2.0

use ostd::cpu::context::UserContext;

use super::SyscallReturn;
use crate::prelude::*;

#[expect(non_camel_case_types)]
#[repr(u64)]
#[derive(Debug, TryFromInt)]
enum ArchPrctlCode {
    ARCH_SET_GS = 0x1001,
    ARCH_SET_FS = 0x1002,
    ARCH_GET_FS = 0x1003,
    ARCH_GET_GS = 0x1004,
}

pub fn sys_arch_prctl(
    code: u64,
    addr: u64,
    _ctx: &Context,
    user_ctx: &mut UserContext,
) -> Result<SyscallReturn> {
    let arch_prctl_code = ArchPrctlCode::try_from(code)?;
    debug!(
        "arch_prctl_code: {:?}, addr = 0x{:x}",
        arch_prctl_code, addr
    );
    let res = do_arch_prctl(arch_prctl_code, addr, user_ctx).unwrap();
    Ok(SyscallReturn::Return(res as _))
}

fn do_arch_prctl(code: ArchPrctlCode, addr: u64, user_ctx: &mut UserContext) -> Result<u64> {
    match code {
        ArchPrctlCode::ARCH_SET_FS => {
            user_ctx.set_tls_pointer(addr as usize);
            user_ctx.activate_tls_pointer();
            Ok(0)
        }
        ArchPrctlCode::ARCH_GET_FS => Ok(user_ctx.tls_pointer() as u64),
        ArchPrctlCode::ARCH_GET_GS | ArchPrctlCode::ARCH_SET_GS => {
            return_errno_with_message!(Errno::EINVAL, "GS cannot be accessed from the user space")
        }
    }
}
