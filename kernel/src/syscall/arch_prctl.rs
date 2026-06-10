// SPDX-License-Identifier: MPL-2.0

use ostd::{
    arch::cpu::context::{FsBase, GsBase},
    mm::{MAX_USERSPACE_VADDR, VmIo},
};

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

pub fn sys_arch_prctl(code: u64, addr: usize, ctx: &Context) -> Result<SyscallReturn> {
    let arch_prctl_code = ArchPrctlCode::try_from(code)?;
    debug!(
        "arch_prctl_code: {:?}, addr = 0x{:x}",
        arch_prctl_code, addr
    );
    let res = do_arch_prctl(arch_prctl_code, addr, ctx)?;
    Ok(SyscallReturn::Return(res as _))
}

fn do_arch_prctl(code: ArchPrctlCode, addr: usize, ctx: &Context) -> Result<u64> {
    let supp = ctx.thread_local.supp_user_context();

    match code {
        ArchPrctlCode::ARCH_SET_GS => {
            if addr >= MAX_USERSPACE_VADDR {
                return_errno_with_message!(Errno::EPERM, "gsbase must be a userspace address");
            }
            supp.gs_base().set(GsBase::new(addr));
            Ok(0)
        }
        ArchPrctlCode::ARCH_SET_FS => {
            if addr >= MAX_USERSPACE_VADDR {
                return_errno_with_message!(Errno::EPERM, "fsbase must be a userspace address");
            }
            supp.fs_base().set(FsBase::new(addr));
            Ok(0)
        }
        ArchPrctlCode::ARCH_GET_FS => {
            let base = supp.fs_base().get().addr();
            ctx.user_space().write_val(addr, &base)?;
            Ok(0)
        }
        ArchPrctlCode::ARCH_GET_GS => {
            let base = supp.gs_base().get().addr();
            ctx.user_space().write_val(addr, &base)?;
            Ok(0)
        }
    }
}
