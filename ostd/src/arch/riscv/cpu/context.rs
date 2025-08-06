// SPDX-License-Identifier: MPL-2.0

//! CPU execution context control.

use core::fmt::Debug;

use riscv::register::scause::{Exception, Trap};

use crate::{
    arch::trap::{call_irq_callback_functions_by_scause, RawUserContext, TrapFrame},
    cpu::PrivilegeLevel,
    user::{ReturnReason, UserContextApi, UserContextApiInternal},
};

/// Userspace CPU context, including general-purpose registers and exception information.
#[derive(Clone, Default, Debug)]
#[repr(C)]
pub struct UserContext {
    user_context: RawUserContext,
    exception: Option<CpuException>,
}

/// General registers.
#[derive(Debug, Default, Clone, Copy)]
#[repr(C)]
#[expect(missing_docs)]
pub struct GeneralRegs {
    pub zero: usize,
    pub ra: usize,
    pub sp: usize,
    pub gp: usize,
    pub tp: usize,
    pub t0: usize,
    pub t1: usize,
    pub t2: usize,
    pub s0: usize,
    pub s1: usize,
    pub a0: usize,
    pub a1: usize,
    pub a2: usize,
    pub a3: usize,
    pub a4: usize,
    pub a5: usize,
    pub a6: usize,
    pub a7: usize,
    pub s2: usize,
    pub s3: usize,
    pub s4: usize,
    pub s5: usize,
    pub s6: usize,
    pub s7: usize,
    pub s8: usize,
    pub s9: usize,
    pub s10: usize,
    pub s11: usize,
    pub t3: usize,
    pub t4: usize,
    pub t5: usize,
    pub t6: usize,
}

/// RISC-V CPU exceptions
///
/// Every enum variant corresponds to one exception defined by the RISC-V
/// architecture. Variants that naturally carry an error code (in stval
/// register) expose it through their associated data fields.
#[derive(Clone, Copy, Debug)]
pub enum CpuException {
    /// Instruction address misalignment exception.
    InstructionMisaligned,
    /// Instruction access fault exception.
    InstructionFault,
    /// Illegal instruction exception.
    IllegalInstruction(FaultInstruction),
    /// Breakpoint exception.
    Breakpoint,
    /// Load address misalignment exception.
    LoadMisaligned(FaultAddress),
    /// Load access fault exception.
    LoadFault(FaultAddress),
    /// Store address misalignment exception.
    StoreMisaligned(FaultAddress),
    /// Store access fault exception.
    StoreFault(FaultAddress),
    /// Environment call from user mode.
    UserEnvCall,
    /// Environment call from supervisor mode.
    SupervisorEnvCall,
    /// Instruction page fault exception.
    InstructionPageFault(FaultAddress),
    /// Load page fault exception.
    LoadPageFault(FaultAddress),
    /// Store page fault exception.
    StorePageFault(FaultAddress),
    /// Unknown.
    Unknown,
}

impl CpuException {
    pub(in crate::arch) fn new(raw_exception: Exception, stval: usize) -> Self {
        use Exception::*;

        match raw_exception {
            InstructionMisaligned => Self::InstructionMisaligned,
            InstructionFault => Self::InstructionFault,
            IllegalInstruction => Self::IllegalInstruction({
                if stval & 0x3 == 0x3 {
                    FaultInstruction::Normal(stval as u32)
                } else {
                    FaultInstruction::Compressed(stval as u16)
                }
            }),
            Breakpoint => Self::Breakpoint,
            LoadMisaligned => Self::LoadMisaligned(stval),
            LoadFault => Self::LoadFault(stval),
            StoreMisaligned => Self::StoreMisaligned(stval),
            StoreFault => Self::StoreFault(stval),
            UserEnvCall => Self::UserEnvCall,
            SupervisorEnvCall => Self::SupervisorEnvCall,
            InstructionPageFault => Self::InstructionPageFault(stval),
            LoadPageFault => Self::LoadPageFault(stval),
            StorePageFault => Self::StorePageFault(stval),
            Unknown => Self::Unknown,
        }
    }
}

/// Data address of data access exceptions.
pub type FaultAddress = usize;

/// Illegal instruction that caused exception.
#[derive(Clone, Copy, Debug)]
pub enum FaultInstruction {
    /// Normal 4-byte instruction.
    Normal(u32),
    /// Compressed 2-byte instruction. Used only when compressed (C) extension
    /// is enabled.
    Compressed(u16),
}

impl UserContext {
    /// Returns a reference to the general registers.
    pub fn general_regs(&self) -> &GeneralRegs {
        &self.user_context.general
    }

    /// Returns a mutable reference to the general registers
    pub fn general_regs_mut(&mut self) -> &mut GeneralRegs {
        &mut self.user_context.general
    }

    /// Takes the CPU exception out.
    pub fn take_exception(&mut self) -> Option<CpuException> {
        self.exception.take()
    }

    /// Sets the thread-local storage pointer.
    pub fn set_tls_pointer(&mut self, tls: usize) {
        self.set_tp(tls)
    }

    /// Gets the thread-local storage pointer.
    pub fn tls_pointer(&self) -> usize {
        self.tp()
    }

    /// Activates the thread-local storage pointer for the current task.
    pub fn activate_tls_pointer(&self) {
        // In RISC-V, `tp` will be loaded at `UserContext::execute`, so it does not need to be
        // activated in advance.
    }
}

impl UserContextApiInternal for UserContext {
    fn execute<F>(&mut self, mut has_kernel_event: F) -> ReturnReason
    where
        F: FnMut() -> bool,
    {
        loop {
            crate::task::scheduler::might_preempt();
            self.user_context.run();

            let scause = riscv::register::scause::read();
            match scause.cause() {
                Trap::Interrupt(interrupt) => {
                    call_irq_callback_functions_by_scause(
                        &self.as_trap_frame(),
                        scause.bits(),
                        interrupt,
                        PrivilegeLevel::User,
                    );
                    crate::arch::irq::enable_local();
                }
                Trap::Exception(Exception::UserEnvCall) => {
                    crate::arch::irq::enable_local();
                    self.user_context.sepc += 4;
                    break ReturnReason::UserSyscall;
                }
                Trap::Exception(raw_exception) => {
                    let stval = riscv::register::stval::read();
                    crate::arch::irq::enable_local();

                    let exception = CpuException::new(raw_exception, stval);
                    self.exception = Some(exception);
                    break ReturnReason::UserException;
                }
            }

            if has_kernel_event() {
                break ReturnReason::KernelEvent;
            }
        }
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
        self.user_context.sepc = ip;
    }

    fn stack_pointer(&self) -> usize {
        self.sp()
    }

    fn set_stack_pointer(&mut self, sp: usize) {
        self.set_sp(sp);
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

/// The FPU context of user task.
///
/// This could be used for saving both legacy and modern state format.
// FIXME: Implement FPU context on RISC-V platforms.
#[derive(Clone, Debug, Default)]
pub struct FpuContext;

impl FpuContext {
    /// Creates a new FPU context.
    pub fn new() -> Self {
        Self
    }

    /// Saves CPU's current FPU context to this instance, if needed.
    pub fn save(&mut self) {}

    /// Loads CPU's FPU context from this instance, if needed.
    pub fn load(&mut self) {}

    /// Returns the FPU context as a byte slice.
    pub fn as_bytes(&self) -> &[u8] {
        &[]
    }

    /// Returns the FPU context as a mutable byte slice.
    pub fn as_bytes_mut(&mut self) -> &mut [u8] {
        &mut []
    }
}
