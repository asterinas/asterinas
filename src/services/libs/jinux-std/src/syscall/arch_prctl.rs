use jinux_frame::cpu::CpuContext;

use crate::prelude::*;
use crate::syscall::SYS_ARCH_PRCTL;

use super::SyscallReturn;

#[allow(non_camel_case_types)]
#[derive(Debug)]
pub enum ArchPrctlCode {
    ARCH_SET_GS = 0x1001,
    ARCH_SET_FS = 0x1002,
    ARCH_GET_FS = 0x1003,
    ARCH_GET_GS = 0x1004,
}

impl TryFrom<u64> for ArchPrctlCode {
    type Error = Error;

    fn try_from(value: u64) -> Result<Self> {
        match value {
            0x1001 => Ok(ArchPrctlCode::ARCH_SET_GS),
            0x1002 => Ok(ArchPrctlCode::ARCH_SET_FS),
            0x1003 => Ok(ArchPrctlCode::ARCH_GET_FS),
            0x1004 => Ok(ArchPrctlCode::ARCH_GET_GS),
            _ => return_errno_with_message!(Errno::EINVAL, "Unknown code for arch_prctl"),
        }
    }
}

pub fn sys_arch_prctl(code: u64, addr: u64, context: &mut CpuContext) -> Result<SyscallReturn> {
    debug!("[syscall][id={}][SYS_ARCH_PRCTL]", SYS_ARCH_PRCTL);
    let arch_prctl_code = ArchPrctlCode::try_from(code)?;
    debug!(
        "arch_prctl_code: {:?}, addr = 0x{:x}",
        arch_prctl_code, addr
    );
    let res = do_arch_prctl(arch_prctl_code, addr, context).unwrap();
    Ok(SyscallReturn::Return(res as _))
}

pub fn do_arch_prctl(code: ArchPrctlCode, addr: u64, context: &mut CpuContext) -> Result<u64> {
    match code {
        ArchPrctlCode::ARCH_SET_FS => {
            context.fs_base = addr;
            Ok(0)
        }
        ArchPrctlCode::ARCH_GET_FS => Ok(context.fs_base),
        ArchPrctlCode::ARCH_GET_GS | ArchPrctlCode::ARCH_SET_GS => {
            return_errno_with_message!(Errno::EINVAL, "GS cannot be accessed from the user space")
        }
    }
}
