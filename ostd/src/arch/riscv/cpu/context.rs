// SPDX-License-Identifier: MPL-2.0

//! CPU execution context control.

use core::{arch::asm, default, fmt::Debug};

use riscv::register::scause::{Exception, Interrupt, Trap};

pub use crate::arch::trap::GeneralRegs as RawGeneralRegs;
use crate::{
    arch::{
        kernel::plic::claim_interrupt,
        timer::handle_timer_interrupt,
        trap::{TrapFrame, UserContext as RawUserContext},
    },
    cpu::current_cpu_racy,
    trap::call_irq_callback_functions,
    user::{ReturnReason, UserContextApi, UserContextApiInternal},
};

/// Cpu context, including both general-purpose registers and FPU state.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct UserContext {
    user_context: RawUserContext,
    trap: Trap,
    fpu_state: FpuState,
    cpu_exception_info: CpuExceptionInfo,
}

/// CPU exception information.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct CpuExceptionInfo {
    /// The type of the exception.
    pub code: Exception,
    /// The virtual address where a page fault occurred.
    pub page_fault_addr: usize,
    /// The error code associated with the exception.
    pub error_code: usize, // TODO
    /// The illegal instruction.
    pub illegal_instruction: usize,
}

impl Default for UserContext {
    fn default() -> Self {
        UserContext {
            user_context: RawUserContext::default(),
            trap: Trap::Exception(Exception::Unknown),
            fpu_state: FpuState::default(),
            cpu_exception_info: CpuExceptionInfo::default(),
        }
    }
}

impl Default for CpuExceptionInfo {
    fn default() -> Self {
        CpuExceptionInfo {
            code: Exception::Unknown,
            page_fault_addr: 0,
            error_code: 0,
            illegal_instruction: 0,
        }
    }
}

impl CpuExceptionInfo {
    /// Get corresponding CPU exception
    pub fn cpu_exception(&self) -> CpuException {
        self.code
    }
}

impl UserContext {
    /// Returns a reference to the general registers.
    pub fn general_regs(&self) -> &RawGeneralRegs {
        &self.user_context.general
    }

    /// Returns a mutable reference to the general registers
    pub fn general_regs_mut(&mut self) -> &mut RawGeneralRegs {
        &mut self.user_context.general
    }

    /// Returns the trap information.
    pub fn trap_information(&self) -> &CpuExceptionInfo {
        &self.cpu_exception_info
    }

    /// Returns a reference to the FPU state.
    pub fn fpu_state(&self) -> &FpuState {
        &self.fpu_state
    }

    /// Returns a mutable reference to the FPU state.
    pub fn fpu_state_mut(&mut self) -> &mut FpuState {
        &mut self.fpu_state
    }

    /// Sets thread-local storage pointer.
    pub fn set_tls_pointer(&mut self, tls: usize) {
        self.set_tp(tls)
    }

    /// Gets thread-local storage pointer.
    pub fn tls_pointer(&self) -> usize {
        self.tp()
    }

    /// Activates thread-local storage pointer on the current CPU.
    pub fn activate_tls_pointer(&self) {
        // No-op
    }

    /// Initializes FPU state.
    pub fn init_fpu_state(&self) {
        // We need to set sstatus.FS to Init, but self is not mutable.
        // So we use unsafe block to modify sstatus.
        unsafe {
            let sstatus_ptr = core::ptr::addr_of!(self.user_context.sstatus) as *mut usize;
            let current = sstatus_ptr.read();
            let modified = (current & !(0b11 << 13)) | (0b01 << 13);
            sstatus_ptr.write(modified);
        }
    }
}

impl UserContextApiInternal for UserContext {
    fn execute<F>(&mut self, mut has_kernel_event: F) -> ReturnReason
    where
        F: FnMut() -> bool,
    {
        let ret = loop {
            self.user_context.run();
            match riscv::register::scause::read().cause() {
                Trap::Interrupt(Interrupt::SupervisorTimer) => {
                    handle_timer_interrupt();
                }
                Trap::Interrupt(Interrupt::SupervisorExternal) => {
                    while let irq_num = claim_interrupt(current_cpu_racy().as_usize())
                        && irq_num != 0
                    {
                        call_irq_callback_functions(&self.as_trap_frame(), irq_num);
                    }
                }
                Trap::Interrupt(_) => todo!(),
                Trap::Exception(Exception::UserEnvCall) => {
                    self.user_context.sepc += 4;
                    break ReturnReason::UserSyscall;
                }
                Trap::Exception(e) => {
                    let stval = riscv::register::stval::read();
                    let sepc = riscv::register::sepc::read();
                    log::trace!("Exception, scause: {e:?}, stval: {stval:#x?}, sepc: {sepc:#x?}");
                    match e {
                        // Check if the exception is a page fault
                        // If so, the address is stored in stval
                        Exception::StorePageFault
                        | Exception::LoadPageFault
                        | Exception::InstructionPageFault => {
                            self.cpu_exception_info = CpuExceptionInfo {
                                code: e,
                                page_fault_addr: stval,
                                error_code: 0,
                                illegal_instruction: 0,
                            };
                        }
                        // Check if the exception is a illegal instruction fault
                        // If so, the instruction is stored in stval
                        Exception::IllegalInstruction => {
                            self.cpu_exception_info = CpuExceptionInfo {
                                code: e,
                                page_fault_addr: sepc,
                                error_code: 0,
                                illegal_instruction: stval,
                            };
                        }
                        _ => {
                            self.cpu_exception_info = CpuExceptionInfo {
                                code: e,
                                page_fault_addr: 0,
                                error_code: 0,
                                illegal_instruction: 0,
                            }
                        }
                    }
                    break ReturnReason::UserException;
                }
            }

            if has_kernel_event() {
                break ReturnReason::KernelEvent;
            }
        };

        crate::arch::irq::enable_local();
        ret
    }

    fn as_trap_frame(&self) -> TrapFrame {
        TrapFrame {
            general: self.user_context.general,
            sstatus: self.user_context.sstatus,
            sepc: self.user_context.sepc,
        }
    }
}

impl UserContextApi for UserContext {
    fn trap_number(&self) -> usize {
        todo!()
    }

    fn trap_error_code(&self) -> usize {
        todo!()
    }

    fn instruction_pointer(&self) -> usize {
        self.user_context.sepc
    }

    fn set_instruction_pointer(&mut self, ip: usize) {
        self.user_context.set_ip(ip);
    }

    fn stack_pointer(&self) -> usize {
        self.user_context.get_sp()
    }

    fn set_stack_pointer(&mut self, sp: usize) {
        self.user_context.set_sp(sp);
    }
}

macro_rules! cpu_context_impl_getter_setter {
    ( $( [ $field: ident, $setter_name: ident] ),*) => {
        impl UserContext {
            $(
                #[doc = concat!("Gets the value of ", stringify!($field))]
                #[inline(always)]
                pub fn $field(&self) -> usize {
                    self.user_context.general.$field
                }

                #[doc = concat!("Sets the value of ", stringify!($field))]
                #[inline(always)]
                pub fn $setter_name(&mut self, $field: usize) {
                    self.user_context.general.$field = $field;
                }
            )*
        }
    };
}

cpu_context_impl_getter_setter!(
    [ra, set_ra],
    [sp, set_sp],
    [gp, set_gp],
    [tp, set_tp],
    [t0, set_t0],
    [t1, set_t1],
    [t2, set_t2],
    [s0, set_s0],
    [s1, set_s1],
    [a0, set_a0],
    [a1, set_a1],
    [a2, set_a2],
    [a3, set_a3],
    [a4, set_a4],
    [a5, set_a5],
    [a6, set_a6],
    [a7, set_a7],
    [s2, set_s2],
    [s3, set_s3],
    [s4, set_s4],
    [s5, set_s5],
    [s6, set_s6],
    [s7, set_s7],
    [s8, set_s8],
    [s9, set_s9],
    [s10, set_s10],
    [s11, set_s11],
    [t3, set_t3],
    [t4, set_t4],
    [t5, set_t5],
    [t6, set_t6]
);

/// CPU exception.
pub type CpuException = Exception;

/// The FPU state of user task.
#[derive(Clone, Copy, Debug, Default)]
pub struct FpuState {
    pub f0: u64,
    pub f1: u64,
    pub f2: u64,
    pub f3: u64,
    pub f4: u64,
    pub f5: u64,
    pub f6: u64,
    pub f7: u64,
    pub f8: u64,
    pub f9: u64,
    pub f10: u64,
    pub f11: u64,
    pub f12: u64,
    pub f13: u64,
    pub f14: u64,
    pub f15: u64,
    pub f16: u64,
    pub f17: u64,
    pub f18: u64,
    pub f19: u64,
    pub f20: u64,
    pub f21: u64,
    pub f22: u64,
    pub f23: u64,
    pub f24: u64,
    pub f25: u64,
    pub f26: u64,
    pub f27: u64,
    pub f28: u64,
    pub f29: u64,
    pub f30: u64,
    pub f31: u64,
    pub fscr: u32,
}

impl FpuState {
    /// Saves CPU's current FPU state into this instance.
    pub fn save(&self) {
        let fs = riscv::register::sstatus::read().fs();

        if fs == riscv::register::sstatus::FS::Dirty {
            let fpu_state = self as *const FpuState as *mut FpuState;

            // SAFETY: Kernel code don't use FPU state.
            unsafe {
                asm!(
                    "fsd f0,  0*8({0})",
                    "fsd f1,  1*8({0})",
                    "fsd f2,  2*8({0})",
                    "fsd f3,  3*8({0})",
                    "fsd f4,  4*8({0})",
                    "fsd f5,  5*8({0})",
                    "fsd f6,  6*8({0})",
                    "fsd f7,  7*8({0})",
                    "fsd f8,  8*8({0})",
                    "fsd f9,  9*8({0})",
                    "fsd f10, 10*8({0})",
                    "fsd f11, 11*8({0})",
                    "fsd f12, 12*8({0})",
                    "fsd f13, 13*8({0})",
                    "fsd f14, 14*8({0})",
                    "fsd f15, 15*8({0})",
                    "fsd f16, 16*8({0})",
                    "fsd f17, 17*8({0})",
                    "fsd f18, 18*8({0})",
                    "fsd f19, 19*8({0})",
                    "fsd f20, 20*8({0})",
                    "fsd f21, 21*8({0})",
                    "fsd f22, 22*8({0})",
                    "fsd f23, 23*8({0})",
                    "fsd f24, 24*8({0})",
                    "fsd f25, 25*8({0})",
                    "fsd f26, 26*8({0})",
                    "fsd f27, 27*8({0})",
                    "fsd f28, 28*8({0})",
                    "fsd f29, 29*8({0})",
                    "fsd f30, 30*8({0})",
                    "fsd f31, 31*8({0})",
                    "frcsr t0",
                    "sw t0, 32*8({0})",
                    in(reg) fpu_state,
                );

                riscv::register::sstatus::set_fs(riscv::register::sstatus::FS::Clean);
            }
        }
    }

    /// Restores CPU's FPU state from this instance.
    pub fn restore(&self) {
        let fs = riscv::register::sstatus::read().fs();

        match fs {
            // Fixme: Normal state should be clean, because the kernel code don't use FPU state.
            // But the logger use the float instructions.
            riscv::register::sstatus::FS::Dirty | riscv::register::sstatus::FS::Clean => {
                let fpu_state = self;

                // SAFETY: Kernel code don't use FPU state.
                unsafe {
                    asm!(
                        "fld f0,  0*8({0})",
                        "fld f1,  1*8({0})",
                        "fld f2,  2*8({0})",
                        "fld f3,  3*8({0})",
                        "fld f4,  4*8({0})",
                        "fld f5,  5*8({0})",
                        "fld f6,  6*8({0})",
                        "fld f7,  7*8({0})",
                        "fld f8,  8*8({0})",
                        "fld f9,  9*8({0})",
                        "fld f10, 10*8({0})",
                        "fld f11, 11*8({0})",
                        "fld f12, 12*8({0})",
                        "fld f13, 13*8({0})",
                        "fld f14, 14*8({0})",
                        "fld f15, 15*8({0})",
                        "fld f16, 16*8({0})",
                        "fld f17, 17*8({0})",
                        "fld f18, 18*8({0})",
                        "fld f19, 19*8({0})",
                        "fld f20, 20*8({0})",
                        "fld f21, 21*8({0})",
                        "fld f22, 22*8({0})",
                        "fld f23, 23*8({0})",
                        "fld f24, 24*8({0})",
                        "fld f25, 25*8({0})",
                        "fld f26, 26*8({0})",
                        "fld f27, 27*8({0})",
                        "fld f28, 28*8({0})",
                        "fld f29, 29*8({0})",
                        "fld f30, 30*8({0})",
                        "fld f31, 31*8({0})",
                        "lw t0, 32*8({0})",
                        "fscsr t0",
                        in(reg) fpu_state,
                    );
                }
            }
            riscv::register::sstatus::FS::Initial => {
                // SAFETY: Kernel code don't use FPU state.
                unsafe {
                    asm!(
                        "fmv.d.x f0, zero",
                        "fmv.d.x f1, zero",
                        "fmv.d.x f2, zero",
                        "fmv.d.x f3, zero",
                        "fmv.d.x f4, zero",
                        "fmv.d.x f5, zero",
                        "fmv.d.x f6, zero",
                        "fmv.d.x f7, zero",
                        "fmv.d.x f8, zero",
                        "fmv.d.x f9, zero",
                        "fmv.d.x f10, zero",
                        "fmv.d.x f11, zero",
                        "fmv.d.x f12, zero",
                        "fmv.d.x f13, zero",
                        "fmv.d.x f14, zero",
                        "fmv.d.x f15, zero",
                        "fmv.d.x f16, zero",
                        "fmv.d.x f17, zero",
                        "fmv.d.x f18, zero",
                        "fmv.d.x f19, zero",
                        "fmv.d.x f20, zero",
                        "fmv.d.x f21, zero",
                        "fmv.d.x f22, zero",
                        "fmv.d.x f23, zero",
                        "fmv.d.x f24, zero",
                        "fmv.d.x f25, zero",
                        "fmv.d.x f26, zero",
                        "fmv.d.x f27, zero",
                        "fmv.d.x f28, zero",
                        "fmv.d.x f29, zero",
                        "fmv.d.x f30, zero",
                        "fmv.d.x f31, zero",
                        "fscsr zero",
                    );
                    riscv::register::sstatus::set_fs(riscv::register::sstatus::FS::Initial);
                }
            }
            riscv::register::sstatus::FS::Off => {}
        }
    }
}
