// SPDX-License-Identifier: MPL-2.0

use aster_frame::cpu::UserContext;

use crate::syscall::SYS_ARCH_PRCTL;
use crate::{log_syscall_entry, prelude::*};

use super::SyscallReturn;

#[allow(non_camel_case_types)]
#[repr(u64)]
#[derive(Debug, TryFromInt)]
pub enum ArchPrctlCode {
    ARCH_SET_GS = 0x1001,
    ARCH_SET_FS = 0x1002,
    ARCH_GET_FS = 0x1003,
    ARCH_GET_GS = 0x1004,
}

pub fn sys_arch_prctl(code: u64, addr: u64, context: &mut UserContext) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_ARCH_PRCTL);
    let arch_prctl_code = ArchPrctlCode::try_from(code)?;
    debug!(
        "arch_prctl_code: {:?}, addr = 0x{:x}",
        arch_prctl_code, addr
    );
    let res = do_arch_prctl(arch_prctl_code, addr, context).unwrap();
    Ok(SyscallReturn::Return(res as _))
}

pub fn do_arch_prctl(code: ArchPrctlCode, addr: u64, context: &mut UserContext) -> Result<u64> {
    match code {
        ArchPrctlCode::ARCH_SET_FS => {
            context.set_fsbase(addr as usize);
            Ok(0)
        }
        ArchPrctlCode::ARCH_GET_FS => Ok(context.fsbase() as u64),
        ArchPrctlCode::ARCH_GET_GS | ArchPrctlCode::ARCH_SET_GS => {
            return_errno_with_message!(Errno::EINVAL, "GS cannot be accessed from the user space")
        }
    }
}
