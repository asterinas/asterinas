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
        // ARM64 Linux ABI: x8 holds syscall number
        self.x8()
    }

    fn syscall_ret(&self) -> usize {
        // ARM64 Linux ABI: x0 holds return value
        self.x0()
    }

    fn set_syscall_ret(&mut self, ret: usize) {
        self.set_x0(ret)
    }

    fn syscall_args(&self) -> [usize; 6] {
        // ARM64 Linux ABI: x0-x5 hold syscall arguments
        [
            self.x0(),
            self.x1(),
            self.x2(),
            self.x3(),
            self.x4(),
            self.x5(),
        ]
    }
}

macro_rules! copy_gp_regs {
    ($src: ident, $dst: ident) => {
        $dst.x0 = $src.x0;
        $dst.x1 = $src.x1;
        $dst.x2 = $src.x2;
        $dst.x3 = $src.x3;
        $dst.x4 = $src.x4;
        $dst.x5 = $src.x5;
        $dst.x6 = $src.x6;
        $dst.x7 = $src.x7;
        $dst.x8 = $src.x8;
        $dst.x9 = $src.x9;
        $dst.x10 = $src.x10;
        $dst.x11 = $src.x11;
        $dst.x12 = $src.x12;
        $dst.x13 = $src.x13;
        $dst.x14 = $src.x14;
        $dst.x15 = $src.x15;
        $dst.x16 = $src.x16;
        $dst.x17 = $src.x17;
        $dst.x18 = $src.x18;
        $dst.x19 = $src.x19;
        $dst.x20 = $src.x20;
        $dst.x21 = $src.x21;
        $dst.x22 = $src.x22;
        $dst.x23 = $src.x23;
        $dst.x24 = $src.x24;
        $dst.x25 = $src.x25;
        $dst.x26 = $src.x26;
        $dst.x27 = $src.x27;
        $dst.x28 = $src.x28;
        $dst.x29 = $src.x29;
        $dst.x30 = $src.x30;
        $dst.sp = $src.sp;
    };
}

/// Represents the context of a signal handler.
///
/// This contains the context saved before a signal handler is invoked; it will be restored by
/// `sys_rt_sigreturn`.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.15.7/source/arch/arm64/include/uapi/asm/sigcontext.h>
#[repr(C)]
#[repr(align(16))]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub struct SigContext {
    x0: usize,
    x1: usize,
    x2: usize,
    x3: usize,
    x4: usize,
    x5: usize,
    x6: usize,
    x7: usize,
    x8: usize,
    x9: usize,
    x10: usize,
    x11: usize,
    x12: usize,
    x13: usize,
    x14: usize,
    x15: usize,
    x16: usize,
    x17: usize,
    x18: usize,
    x19: usize,
    x20: usize,
    x21: usize,
    x22: usize,
    x23: usize,
    x24: usize,
    x25: usize,
    x26: usize,
    x27: usize,
    x28: usize,
    x29: usize,
    x30: usize,
    sp: usize,
    pc: usize,
    pstate: usize,
    fault_address: usize,
    reserved: [u64; 1],
}

impl SigContext {
    pub fn copy_user_regs_to(&self, dst: &mut UserContext) {
        let gp_regs = dst.general_regs_mut();
        copy_gp_regs!(self, gp_regs);
        dst.set_instruction_pointer(self.pc);
        dst.set_spsr_el1(self.pstate);
    }

    pub fn copy_user_regs_from(&mut self, src: &UserContext) {
        let gp_regs = src.general_regs();
        copy_gp_regs!(gp_regs, self);
        self.pc = src.instruction_pointer();
        self.pstate = src.spsr_el1();
    }
}

/// Reads the TPIDR_EL0 register (user-space TLS pointer).
///
/// This is used during context switching to save the current thread's TLS value
/// from the hardware register.
#[expect(unsafe_code)]
pub fn read_tpidr_el0() -> usize {
    let val: usize;
    // SAFETY: Reading TPIDR_EL0 is safe and doesn't affect kernel state.
    unsafe { core::arch::asm!("mrs {0}, tpidr_el0", out(reg) val) };
    val
}

/// Writes the TPIDR_EL0 register (user-space TLS pointer).
///
/// This is used during context switching to restore the next thread's TLS value
/// into the hardware register. On AArch64, EL0 can also write TPIDR_EL0 directly
/// via `msr` instruction — the kernel does not intercept or override user-space writes.
#[expect(unsafe_code)]
pub fn write_tpidr_el0(val: usize) {
    // SAFETY: Writing TPIDR_EL0 is safe. It only affects the current thread's
    // user-space TLS pointer and won't affect kernel code.
    unsafe { core::arch::asm!("msr tpidr_el0, {0}", in(reg) val) };
}

impl TryFrom<&CpuException> for PageFaultInfo {
    type Error = ();

    fn try_from(value: &CpuException) -> Result<Self, ()> {
        use CpuException::*;

        let (fault_addr, required_perms) = match value {
            InstructionPageFault(addr) => (addr, VmPerms::EXEC),
            LoadPageFault(addr) => (addr, VmPerms::READ),
            // On ARM64, writable pages must also be readable.
            StorePageFault(addr) => (addr, VmPerms::READ | VmPerms::WRITE),
            _ => return Err(()),
        };
        Ok(PageFaultInfo::new(*fault_addr, required_perms))
    }
}

/// CPU information to be shown in `/proc/cpuinfo`.
pub struct CpuInformation {
    processor: u32,
}

impl CpuInformation {
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
