// SPDX-License-Identifier: MPL-2.0

use core::fmt;

use ostd::{
    arch::cpu::context::{CpuException, UserContext},
    cpu::PinCurrentCpu,
    task::DisabledPreemptGuard,
    user::UserContextApi,
};

use crate::{
    cpu::LinuxAbi,
    vm::{perms::VmPerms, vmar::PageFaultInfo},
};

impl LinuxAbi for UserContext {
    fn syscall_num(&self) -> usize {
        // The AArch64 Linux syscall number is passed in `x8`.
        self.x(8)
    }

    fn syscall_ret(&self) -> usize {
        self.x(0)
    }

    fn set_syscall_ret(&mut self, ret: usize) {
        self.set_x(0, ret)
    }

    fn syscall_args(&self) -> [usize; 6] {
        [
            self.x(0),
            self.x(1),
            self.x(2),
            self.x(3),
            self.x(4),
            self.x(5),
        ]
    }
}

/// Represents the context of a signal handler.
///
/// This contains the context saved before a signal handler is invoked; it will
/// be restored by `sys_rt_sigreturn`.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.15.7/source/arch/arm64/include/uapi/asm/sigcontext.h#L28>
#[repr(C)]
#[repr(align(16))]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub struct SigContext {
    /// The fault address (`sigcontext.fault_address`).
    fault_address: u64,
    /// General-purpose registers `x0`-`x30`.
    regs: [u64; 31],
    /// Stack pointer.
    sp: u64,
    /// Program counter.
    pc: u64,
    /// Processor state (`PSTATE`).
    pstate: u64,
    // Keeps the struct size a multiple of the 16-byte alignment (no implicit
    // padding, required by `Pod`).
    _reserved: u64,
    // The AArch64 signal frame continues with a variable-length record area
    // (`__reserved`) holding the FP/SIMD context. It is handled separately.
}

impl SigContext {
    pub fn copy_user_regs_to(&self, dst: &mut UserContext) {
        let gp_regs = dst.general_regs_mut();
        for (i, reg) in self.regs.iter().enumerate() {
            gp_regs.x[i] = *reg as usize;
        }
        gp_regs.sp = self.sp as usize;
        dst.set_instruction_pointer(self.pc as usize);
        dst.set_pstate(self.pstate as usize);
    }

    pub fn copy_user_regs_from(&mut self, src: &UserContext) {
        let gp_regs = src.general_regs();
        for (i, reg) in gp_regs.x.iter().enumerate() {
            self.regs[i] = *reg as u64;
        }
        self.sp = gp_regs.sp as u64;
        self.pc = src.instruction_pointer() as u64;
        self.pstate = src.pstate() as u64;
    }
}

impl TryFrom<&CpuException> for PageFaultInfo {
    // [`Err`] indicates that the [`CpuException`] is not a page fault, with no
    // additional error information.
    type Error = ();

    fn try_from(value: &CpuException) -> Result<Self, ()> {
        use CpuException::*;

        match value {
            InstructionAbort(info) if info.is_page_fault() => {
                Ok(PageFaultInfo::new(info.far, VmPerms::EXEC))
            }
            DataAbort(info) if info.is_page_fault() => {
                let perms = if info.is_write() {
                    VmPerms::WRITE
                } else {
                    VmPerms::READ
                };
                Ok(PageFaultInfo::new(info.far, perms))
            }
            _ => Err(()),
        }
    }
}

/// CPU information to be shown in `/proc/cpuinfo`.
//
// TODO: Populate with `MIDR_EL1`-derived fields (implementer, part, etc.).
pub struct CpuInformation {
    processor: u32,
}

impl CpuInformation {
    /// Constructs the information for the current CPU.
    pub fn new(guard: &DisabledPreemptGuard) -> Self {
        Self {
            processor: guard.current_cpu().into(),
        }
    }
}

impl fmt::Display for CpuInformation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "processor\t: {}", self.processor)
    }
}
