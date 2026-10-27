// SPDX-License-Identifier: MPL-2.0

//! Native Linux seccomp ABI layouts.
//!
//! Reference: <https://github.com/torvalds/linux/blob/v6.18/include/uapi/linux/seccomp.h>.

use ostd::{arch::cpu::context::UserContext, user::UserContextApi};

use crate::{cpu::LinuxAbi, prelude::*};

pub(super) const MODE_STRICT: u32 = 1;
pub(super) const MODE_FILTER: u32 = 2;
pub(super) const RET_KILL_PROCESS: u32 = 0x8000_0000;
pub(super) const RET_KILL_THREAD: u32 = 0;
pub(super) const RET_ERRNO: u32 = 0x0005_0000;
pub(super) const RET_TRAP: u32 = 0x0003_0000;
pub(super) const RET_TRACE: u32 = 0x7ff0_0000;
pub(super) const RET_LOG: u32 = 0x7ffc_0000;
pub(super) const RET_ALLOW: u32 = 0x7fff_0000;
pub(super) const RET_ACTION_FULL: u32 = 0xffff_0000;
pub(super) const RET_DATA: u32 = 0x0000_ffff;

/// A userspace classic BPF instruction (`struct sock_filter`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(super) struct SockFilter {
    pub code: u16,
    pub jt: u8,
    pub jf: u8,
    pub k: u32,
}

/// A native 64-bit `struct sock_fprog`, including its ABI padding.
#[repr(C)]
#[derive(Clone, Copy, Pod)]
pub(super) struct SockFprog {
    pub len: u16,
    pub padding: [u8; 6],
    pub filter: u64,
}

/// The read-only input to a seccomp program (`struct seccomp_data`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(crate) struct SeccompData {
    nr: i32,
    arch: u32,
    instruction_pointer: u64,
    args: [u64; 6],
}

/// Sizes of the userspace notification ABI structures.
#[repr(C)]
#[derive(Clone, Copy, Pod)]
pub(super) struct SeccompNotifSizes {
    pub seccomp_notif: u16,
    pub seccomp_notif_resp: u16,
    pub seccomp_data: u16,
}

#[repr(C)]
#[derive(Clone, Copy, Pod)]
pub(super) struct SeccompNotif {
    pub id: u64,
    pub pid: u32,
    pub flags: u32,
    pub data: SeccompData,
}

#[repr(C)]
#[derive(Clone, Copy, Pod)]
pub(super) struct SeccompNotifResp {
    pub id: u64,
    pub val: i64,
    pub error: i32,
    pub flags: u32,
}

impl SeccompData {
    pub(super) fn log(&self, ctx: &Context, action: u32) {
        notice!(
            "pid={} tid={} syscall={} arch={:#x} ip={:#x} action={:#x}",
            ctx.process.pid(),
            ctx.posix_thread.tid(),
            self.nr,
            self.arch,
            self.instruction_pointer,
            action,
        );
    }

    pub(super) fn syscall_number(&self) -> i32 {
        self.nr
    }

    pub(super) fn trap_info(&self, reason: u32) -> crate::process::signal::c_types::siginfo_t {
        use crate::process::signal::{
            c_types::siginfo_t,
            constants::{SIGSYS, SYS_SECCOMP},
        };

        let mut info = siginfo_t::new(SIGSYS, SYS_SECCOMP);
        info.si_errno = reason as i32;
        info.set_syscall(self.instruction_pointer as Vaddr, self.nr, self.arch);
        info
    }

    pub(crate) fn from_context(user_ctx: &UserContext, syscall_number: i32) -> Self {
        // AUDIT_ARCH_* describes the syscall ABI, rather than a userspace argument.
        #[cfg(target_arch = "x86_64")]
        const AUDIT_ARCH: u32 = 0xc000_003e;
        #[cfg(target_arch = "riscv64")]
        const AUDIT_ARCH: u32 = 0xc000_00f3;
        #[cfg(target_arch = "loongarch64")]
        const AUDIT_ARCH: u32 = 0xc000_0102;
        #[cfg(target_arch = "aarch64")]
        const AUDIT_ARCH: u32 = 0xc000_00b7;

        Self {
            nr: syscall_number,
            arch: AUDIT_ARCH,
            instruction_pointer: user_ctx.instruction_pointer() as u64,
            args: user_ctx.syscall_args().map(|arg| arg as u64),
        }
    }

    pub(super) fn word(&self, offset: u32) -> u32 {
        let offset = offset as usize;
        let bytes = &self.as_bytes()[offset..offset + size_of::<u32>()];
        u32::from_ne_bytes(bytes.try_into().unwrap())
    }
}

const _: () = {
    assert!(size_of::<SockFilter>() == 8);
    assert!(size_of::<SockFprog>() == 16);
    assert!(core::mem::offset_of!(SockFprog, filter) == 8);
    assert!(size_of::<SeccompData>() == 64);
    assert!(core::mem::offset_of!(SeccompData, args) == 16);
    assert!(size_of::<SeccompNotifSizes>() == 6);
    assert!(size_of::<SeccompNotif>() == 80);
    assert!(size_of::<SeccompNotifResp>() == 24);
};
