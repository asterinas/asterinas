use kxos_frame::{cpu::CpuContext, debug};

use crate::syscall::{SyscallResult, SYS_ARCH_PRCTL};

#[allow(non_camel_case_types)]
pub enum ArchPrctlCode {
    ARCH_SET_GS = 0x1001,
    ARCH_SET_FS = 0x1002,
    ARCH_GET_FS = 0x1003,
    ARCH_GET_GS = 0x1004,
}

impl TryFrom<u64> for ArchPrctlCode {
    type Error = &'static str;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        match value {
            0x1001 => Ok(ArchPrctlCode::ARCH_SET_GS),
            0x1002 => Ok(ArchPrctlCode::ARCH_SET_FS),
            0x1003 => Ok(ArchPrctlCode::ARCH_GET_FS),
            0x1004 => Ok(ArchPrctlCode::ARCH_GET_GS),
            _ => Err("Unknown code for arch_prctl"),
        }
    }
}

pub fn sys_arch_prctl(code: u64, addr: u64, context: &mut CpuContext) -> SyscallResult {
    debug!("[syscall][id={}][SYS_ARCH_PRCTL]", SYS_ARCH_PRCTL);
    let arch_prctl_code = ArchPrctlCode::try_from(code);
    match arch_prctl_code {
        Err(_) => SyscallResult::Return(-1),
        Ok(code) => {
            let res = do_arch_prctl(code, addr, context).unwrap();
            SyscallResult::Return(res as i32)
        }
    }
}

pub fn do_arch_prctl(
    code: ArchPrctlCode,
    addr: u64,
    context: &mut CpuContext,
) -> Result<u64, &'static str> {
    match code {
        ArchPrctlCode::ARCH_SET_FS => {
            debug!("set user fs: 0x{:x}", addr);
            context.fs_base = addr;
            Ok(0)
        }
        ArchPrctlCode::ARCH_GET_FS => Ok(context.fs_base),
        ArchPrctlCode::ARCH_GET_GS | ArchPrctlCode::ARCH_SET_GS => {
            Err("GS cannot be accessed from the user space")
        }
    }
}
