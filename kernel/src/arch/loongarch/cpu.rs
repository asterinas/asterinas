// SPDX-License-Identifier: MPL-2.0

use alloc::{format, string::String};

use ostd::{
    cpu::context::{CpuExceptionInfo, GeneralRegs, UserContext},
    user::UserContextApi,
    Pod,
};

use crate::{cpu::LinuxAbi, thread::exception::PageFaultInfo, vm::perms::VmPerms};

impl LinuxAbi for UserContext {
    fn syscall_num(&self) -> usize {
        self.a7()
    }

    fn set_syscall_num(&mut self, num: usize) {
        self.set_a7(num);
    }

    fn syscall_ret(&self) -> usize {
        self.a0()
    }

    fn set_syscall_ret(&mut self, ret: usize) {
        self.set_a0(ret)
    }

    fn syscall_args(&self) -> [usize; 6] {
        [
            self.a0(),
            self.a1(),
            self.a2(),
            self.a3(),
            self.a4(),
            self.a5(),
        ]
    }
}

/// Represents the context of a signal handler.
///
/// This contains the context saved before a signal handler is invoked; it will be restored by
/// `sys_rt_sigreturn`.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.15.7/source/arch/loongarch/include/uapi/asm/sigcontext.h#L20>
#[repr(C)]
#[repr(align(16))]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub struct SigContext {
    pub pc: usize,
    pub zero: usize,
    pub ra: usize,
    pub tp: usize,
    pub sp: usize,
    pub a0: usize,
    pub a1: usize,
    pub a2: usize,
    pub a3: usize,
    pub a4: usize,
    pub a5: usize,
    pub a6: usize,
    pub a7: usize,
    pub t0: usize,
    pub t1: usize,
    pub t2: usize,
    pub t3: usize,
    pub t4: usize,
    pub t5: usize,
    pub t6: usize,
    pub t7: usize,
    pub t8: usize,
    pub r21: usize,
    pub fp: usize,
    pub s0: usize,
    pub s1: usize,
    pub s2: usize,
    pub s3: usize,
    pub s4: usize,
    pub s5: usize,
    pub s6: usize,
    pub s7: usize,
    pub s8: usize,
    pub flags: u32,
    // In LoongArch, the signal stack layout places the FPU context directly
    // after the general-purpose registers.
    // Reference: <https://elixir.bootlin.com/linux/v6.15.7/source/arch/loongarch/kernel/signal.c#L861>
}

macro_rules! copy_gp_regs {
    ($src: ident, $dst: ident) => {
        $dst.zero = $src.zero;
        $dst.ra = $src.ra;
        $dst.tp = $src.tp;
        $dst.sp = $src.sp;
        $dst.a0 = $src.a0;
        $dst.a1 = $src.a1;
        $dst.a2 = $src.a2;
        $dst.a3 = $src.a3;
        $dst.a4 = $src.a4;
        $dst.a5 = $src.a5;
        $dst.a6 = $src.a6;
        $dst.a7 = $src.a7;
        $dst.t0 = $src.t0;
        $dst.t1 = $src.t1;
        $dst.t2 = $src.t2;
        $dst.t3 = $src.t3;
        $dst.t4 = $src.t4;
        $dst.t5 = $src.t5;
        $dst.t6 = $src.t6;
        $dst.t7 = $src.t7;
        $dst.t8 = $src.t8;
        $dst.r21 = $src.r21;
        $dst.fp = $src.fp;
        $dst.s0 = $src.s0;
        $dst.s1 = $src.s1;
        $dst.s2 = $src.s2;
        $dst.s3 = $src.s3;
        $dst.s4 = $src.s4;
        $dst.s5 = $src.s5;
        $dst.s6 = $src.s6;
        $dst.s7 = $src.s7;
        $dst.s8 = $src.s8;
    };
}

impl SigContext {
    pub fn copy_user_regs_to(&self, dst: &mut UserContext) {
        let gp_regs = dst.general_regs_mut();
        copy_gp_regs!(self, gp_regs);
        dst.set_instruction_pointer(self.pc);
    }

    pub fn copy_user_regs_from(&mut self, src: &UserContext) {
        let gp_regs = src.general_regs();
        copy_gp_regs!(gp_regs, self);
        self.pc = src.instruction_pointer();
    }
}

impl TryFrom<&CpuExceptionInfo> for PageFaultInfo {
    // [`Err`] indicates that the [`CpuExceptionInfo`] is not a page fault,
    // with no additional error information.
    type Error = ();

    fn try_from(value: &CpuExceptionInfo) -> Result<Self, ()> {
        use loongArch64::register::estat::Exception;

        let required_perms = match value.cpu_exception() {
            Exception::FetchPageFault => VmPerms::EXEC,
            Exception::LoadPageFault => VmPerms::READ,
            Exception::StorePageFault => VmPerms::WRITE,
            _ => return Err(()),
        };

        Ok(PageFaultInfo {
            address: value.page_fault_addr,
            required_perms,
        })
    }
}

/// CPU Information structure.
// TODO: Implement CPU information retrieval on LoongArch platforms.
pub struct CpuInfo {
    processor: u32,
}

impl CpuInfo {
    pub fn new(processor_id: u32) -> Self {
        Self {
            processor: processor_id,
        }
    }

    /// Collect and format CPU information into a `String`.
    pub fn collect_cpu_info(&self) -> String {
        format!("processor\t: {}\n", self.processor)
    }
}
