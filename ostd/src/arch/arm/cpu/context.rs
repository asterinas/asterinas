// SPDX-License-Identifier: MPL-2.0

//! CPU execution context control.

use core::{arch::asm, fmt::Debug};

use crate::{
    arch::trap::{RawUserContext, TrapFrame},
    user::{ReturnReason, UserContextApi, UserContextApiInternal, UserModeHooks},
};

/// Userspace CPU context, including general-purpose registers and exception information.
#[repr(C)]
#[derive(Clone, Debug, Default)]
pub struct UserContext {
    user_context: RawUserContext,
    exception: Option<CpuException>,
}

/// General registers.
#[expect(missing_docs)]
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct GeneralRegs {
    pub x1: usize,
    pub x2: usize,
    pub x3: usize,
    pub x4: usize,
    pub x5: usize,
    pub x6: usize,
    pub x7: usize,
    pub x8: usize,
    pub x9: usize,
    pub x10: usize,
    pub x11: usize,
    pub x12: usize,
    pub x13: usize,
    pub x14: usize,
    pub x15: usize,
    pub x16: usize,
    pub x17: usize,
    pub x18: usize,
    pub x19: usize,
    pub x20: usize,
    pub x21: usize,
    pub x22: usize,
    pub x23: usize,
    pub x24: usize,
    pub x25: usize,
    pub x26: usize,
    pub x27: usize,
    pub x28: usize,
    pub x29: usize,
    pub __reserved: usize, // for alignment
    pub x30: usize,
    // put here deliberately for ease of asm
    pub x0: usize,
    // x31 means special
}

/// ARM CPU exceptions.
///
/// Every enum variant corresponds to one exception defined by the ARM
/// architecture.
#[derive(Clone, Debug)]
pub enum CpuException {
    /// Unknown reason.
    Unknown,
    /// 000001 - Trapped WFI or WFE instruction execution.
    WfiInstruction,
    /// 000111 - Access to Advanced SIMD or floating-point functionality.
    FpuInstruction,
    /// 001110 - Illegal Execution state.
    IllegalState,
    /// 010101 - SVC instruction execution in AArch64 state.
    SvcInstruction,
    /// 011000 - Trapped MSR, MRS, or System instruction execution.
    SystemInstruction,
    /// 10000x - Instruction Abort from a lower or the same Execution level.
    InstructionAbort {
        /// The fault address that caused the exception.
        address: usize,
    },
    /// 100010 - PC alignment fault.
    PcAlignmentFault,
    /// 10010x - Data Abort from a lower or the same Exception level.
    DataAbort {
        /// Whether the exception was generated on a write instruction.
        is_write: bool,
        /// The fault address that caused the exception.
        address: usize,
    },
    /// 100110 - SP alignment fault.
    SpAlignmentFault,
    /// 101100 - Trapped floating-point exception taken from AArch64 state.
    FpuException,
    /// 101111 - SError interrupt.
    SErrorInterrupt,
    /// 11000x - Breakpoint exception from a lower or the same Exception level.
    Breakpoint,
    /// 11001x - Software Step exception from a lower or the same Exception level.
    SoftwareStep,
    /// 11010x - Watchpoint exception from a lower or the same Exception level.
    Watchpoint,
    /// 111100 - BRK instruction execution in AArch64 state.
    BrkInstruction,
}

#[derive(Clone, Debug)]
pub(in crate::arch) enum CpuTrap {
    Exception(CpuException),
    Interrupt,
    FastInterrupt,
    SError,
}

impl CpuTrap {
    pub(in crate::arch) fn new(trap_num: usize) -> Option<Self> {
        match trap_num >> 16 {
            // 0: Synchronous
            0 => (),
            // 1: IRQ or vIRQ
            1 => return Some(Self::Interrupt),
            // 2: FIQ or vFIQ
            2 => return Some(Self::FastInterrupt),
            // 3: SError or vSError
            3 => return Some(Self::SError),

            _ => return None,
        }

        let esr_el1: usize;
        // SAFETY: It is safe to read the Exception Syndrome Register (ESR).
        unsafe { asm!("mrs {}, esr_el1", out(reg) esr_el1) };

        fn fault_address() -> usize {
            let far_el1;
            // SAFETY: It is safe to read the Fault Address Register (FAR).
            unsafe { asm!("mrs {}, far_el1", out(reg) far_el1) };
            far_el1
        }

        // WnR, bit [6]: Write not Read.
        const ESR_WNR: usize = 1 << 6;

        // EC, bits[31:26]: The Exception class field.
        let exception = match esr_el1 >> 26 {
            0b000001 => CpuException::WfiInstruction,
            0b000111 => CpuException::FpuInstruction,
            0b001110 => CpuException::IllegalState,
            0b010101 => CpuException::SvcInstruction,
            0b011000 => CpuException::SystemInstruction,
            0b100000 | 0b100001 => CpuException::InstructionAbort {
                address: fault_address(),
            },
            0b100010 => CpuException::PcAlignmentFault,
            0b100100 | 0b100101 => CpuException::DataAbort {
                is_write: esr_el1 & ESR_WNR != 0,
                address: fault_address(),
            },
            0b100110 => CpuException::SpAlignmentFault,
            0b101100 => CpuException::FpuException,
            0b101111 => CpuException::SErrorInterrupt,
            0b110000 | 0b110001 => CpuException::Breakpoint,
            0b110010 | 0b110011 => CpuException::SoftwareStep,
            0b110100 | 0b110101 => CpuException::Watchpoint,
            0b111100 => CpuException::BrkInstruction,

            0b000000 => CpuException::Unknown,
            _ => CpuException::Unknown,
        };
        Some(Self::Exception(exception))
    }
}

impl UserContext {
    // Methods shared across all architectures (i.e., general registers and exceptions).

    /// Returns a reference to the general registers.
    pub fn general_regs(&self) -> &GeneralRegs {
        &self.user_context.general
    }

    /// Returns a mutable reference to the general registers.
    pub fn general_regs_mut(&mut self) -> &mut GeneralRegs {
        &mut self.user_context.general
    }

    /// Takes the CPU exception out.
    pub fn take_exception(&mut self) -> Option<CpuException> {
        self.exception.take()
    }

    // Architecture-specific methods.

    /// Gets the value of the thread-local storage pointer.
    pub fn tls_pointer(&self) -> usize {
        self.user_context.tpidr
    }

    /// Sets the value of the thread-local storage pointer.
    pub fn set_tls_pointer(&mut self, tls: usize) {
        self.user_context.tpidr = tls;
    }

    /// Gets the value of the process state (PSTATE) register.
    pub fn process_state(&self) -> usize {
        self.user_context.spsr
    }

    /// Sets the value of the process state (PSTATE) register.
    ///
    /// We only allow the setting or clearing of arithmetic condition flags and states for Software
    /// Step and Illegal Execution. Any other bits will be ignored and will remain unchanged.
    pub fn set_process_state(&mut self, pstate: usize) {
        // Be careful. We can only allow bits to be set that won't affect soundness.
        const USER_MODIFIABLE_PSTATE: usize =
            // Condition flags: Negative (N), Zero (Z), Carry (C), Overflow (V).
            (0b1111 << 28)
            // Software Step (SS) state.
            | (1 << 21)
            // Illegal Execution (IL) state.
            | (1 << 20);

        self.user_context.spsr =
            (self.user_context.spsr & !USER_MODIFIABLE_PSTATE) | (pstate & USER_MODIFIABLE_PSTATE);
    }
}

impl UserContextApiInternal for UserContext {
    fn execute<T: UserModeHooks>(&mut self, hooks: &T) -> ReturnReason {
        #[expect(clippy::never_loop)] // This will loop once we add support for IRQ handling.
        loop {
            crate::task::scheduler::might_preempt();

            let guard = crate::irq::disable_local();
            hooks.pre_user_run(&guard);
            self.user_context.run(guard);

            let trap = CpuTrap::new(self.user_context.trap_num);
            match trap {
                Some(CpuTrap::Exception(CpuException::SvcInstruction)) => {
                    crate::arch::irq::enable_local();
                    break ReturnReason::UserSyscall;
                }
                Some(CpuTrap::Exception(exception)) => {
                    crate::arch::irq::enable_local();
                    self.exception = Some(exception);
                    break ReturnReason::UserException;
                }
                _ => panic!(
                    "Cannot handle user CPU exception: {:?}; trapframe: {:#?}",
                    trap,
                    self.as_trap_frame()
                ),
            }

            #[expect(unreachable_code)] // This can be reached once we add support for IRQ handling.
            if hooks.has_kernel_event() {
                break ReturnReason::KernelEvent;
            }
        }
    }

    fn as_trap_frame(&self) -> TrapFrame {
        TrapFrame {
            trap_num: self.user_context.trap_num,
            __reserved: self.user_context.__reserved,
            elr: self.user_context.elr,
            spsr: self.user_context.spsr,
            sp: self.user_context.sp,
            tpidr: self.user_context.tpidr,
            general: self.user_context.general,
        }
    }
}

impl UserContextApi for UserContext {
    fn instruction_pointer(&self) -> usize {
        self.user_context.elr
    }

    fn set_instruction_pointer(&mut self, ip: usize) {
        self.user_context.elr = ip;
    }

    fn stack_pointer(&self) -> usize {
        self.user_context.sp
    }

    fn set_stack_pointer(&mut self, sp: usize) {
        self.user_context.sp = sp;
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
    [x0, set_x0],
    [x1, set_x1],
    [x2, set_x2],
    [x3, set_x3],
    [x4, set_x4],
    [x5, set_x5],
    [x6, set_x6],
    [x7, set_x7],
    [x8, set_x8],
    [x9, set_x9],
    [x10, set_x10],
    [x11, set_x11],
    [x12, set_x12],
    [x13, set_x13],
    [x14, set_x14],
    [x15, set_x15],
    [x16, set_x16],
    [x17, set_x17],
    [x18, set_x18],
    [x19, set_x19],
    [x20, set_x20],
    [x21, set_x21],
    [x22, set_x22],
    [x23, set_x23],
    [x24, set_x24],
    [x25, set_x25],
    [x26, set_x26],
    [x27, set_x27],
    [x28, set_x28],
    [x29, set_x29],
    [x30, set_x30]
);
