// SPDX-License-Identifier: MPL-2.0

//! CPU execution context control.

use core::fmt::Debug;

use ostd_pod::IntoBytes;

use crate::{
    arch::{
        irq::handle_irq,
        trap::{RawUserContext, TRAP_KIND_IRQ, TrapFrame},
    },
    cpu::PrivilegeLevel,
    user::{ReturnReason, UserContextApi, UserContextApiInternal},
};

/// General-purpose registers (`x0`-`x30` and the stack pointer).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct GeneralRegs {
    /// `x0`-`x30`.
    pub x: [usize; 31],
    /// Stack pointer (`SP_EL0` for user context, `SP_EL1` for kernel traps).
    pub sp: usize,
}

/// Userspace CPU context, including general-purpose registers and exception
/// information.
#[repr(C)]
#[derive(Clone, Debug, Default)]
pub struct UserContext {
    user_context: RawUserContext,
    exception: Option<CpuException>,
}

/// AArch64 CPU exceptions, decoded from `ESR_EL1`.
#[derive(Clone, Copy, Debug)]
pub enum CpuException {
    /// Supervisor call (`SVC`), i.e. a system call.
    Syscall,
    /// Instruction abort (translation/permission fault while fetching).
    InstructionAbort(FaultInfo),
    /// Data abort (translation/permission fault while accessing data).
    DataAbort(FaultInfo),
    /// PC alignment fault.
    PcAlignment,
    /// SP alignment fault.
    SpAlignment,
    /// Illegal execution state.
    IllegalState,
    /// Software breakpoint (`BRK`).
    Breakpoint,
    /// Any other exception, carrying the raw `ESR_EL1` value.
    Unknown(usize),
}

/// Fault information decoded from `ESR_EL1`/`FAR_EL1`.
#[derive(Clone, Copy, Debug)]
pub struct FaultInfo {
    /// The faulting virtual address (`FAR_EL1`).
    pub far: usize,
    /// The raw exception syndrome (`ESR_EL1`).
    pub esr: usize,
}

impl FaultInfo {
    /// Whether the fault was caused by a write access (data aborts only).
    pub fn is_write(&self) -> bool {
        // ESR.WnR (bit 6) is valid for data aborts with a valid syndrome.
        (self.esr & (1 << 6)) != 0
    }

    /// Whether the fault is a translation or permission fault (as opposed to an
    /// external abort, alignment fault, etc.).
    pub fn is_page_fault(&self) -> bool {
        // DFSC/IFSC is ESR bits [5:0]. Fault status 0b0001xx = translation,
        // 0b0011xx = access flag, 0b0011xx.. permission etc. We treat the
        // translation/access-flag/permission classes as page faults.
        let fsc = self.esr & 0b11_1111;
        matches!(fsc >> 2, 0b0001 | 0b0010 | 0b0011)
    }
}

/// Exception-class field (`ESR_EL1[31:26]`) values we care about.
mod ec {
    pub const SVC64: usize = 0b010101;
    pub const INSN_ABORT_LOWER: usize = 0b100000;
    pub const INSN_ABORT_SAME: usize = 0b100001;
    pub const PC_ALIGNMENT: usize = 0b100010;
    pub const DATA_ABORT_LOWER: usize = 0b100100;
    pub const DATA_ABORT_SAME: usize = 0b100101;
    pub const SP_ALIGNMENT: usize = 0b100110;
    pub const ILLEGAL_STATE: usize = 0b001110;
    pub const BRK64: usize = 0b111100;
}

impl CpuException {
    /// Decodes a CPU exception from the exception syndrome and fault address.
    pub(in crate::arch) fn new(esr: usize, far: usize) -> Self {
        let class = (esr >> 26) & 0b11_1111;
        let info = FaultInfo { far, esr };
        match class {
            ec::SVC64 => Self::Syscall,
            ec::INSN_ABORT_LOWER | ec::INSN_ABORT_SAME => Self::InstructionAbort(info),
            ec::DATA_ABORT_LOWER | ec::DATA_ABORT_SAME => Self::DataAbort(info),
            ec::PC_ALIGNMENT => Self::PcAlignment,
            ec::SP_ALIGNMENT => Self::SpAlignment,
            ec::ILLEGAL_STATE => Self::IllegalState,
            ec::BRK64 => Self::Breakpoint,
            _ => Self::Unknown(esr),
        }
    }

    /// Returns the faulting address if this exception carries one.
    pub fn page_fault_addr(&self) -> Option<usize> {
        match self {
            Self::InstructionAbort(info) | Self::DataAbort(info) if info.is_page_fault() => {
                Some(info.far)
            }
            _ => None,
        }
    }
}

impl UserContext {
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

    /// Sets the thread-local storage pointer (`TPIDR_EL0`).
    pub fn set_tls_pointer(&mut self, tls: usize) {
        self.user_context.tpidr = tls;
    }

    /// Gets the thread-local storage pointer (`TPIDR_EL0`).
    pub fn tls_pointer(&self) -> usize {
        self.user_context.tpidr
    }

    /// Gets the value of register `x[i]`.
    pub fn x(&self, i: usize) -> usize {
        self.user_context.general.x[i]
    }

    /// Sets the value of register `x[i]`.
    pub fn set_x(&mut self, i: usize, val: usize) {
        self.user_context.general.x[i] = val;
    }

    /// Gets the saved program status register (`PSTATE`/`SPSR`).
    pub fn pstate(&self) -> usize {
        self.user_context.spsr
    }

    /// Sets the saved program status register (`PSTATE`/`SPSR`).
    pub fn set_pstate(&mut self, pstate: usize) {
        self.user_context.spsr = pstate;
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

            if self.user_context.trap_kind == TRAP_KIND_IRQ {
                // An interrupt was taken while in userspace. Dispatch it and
                // re-enter userspace unless a kernel event is pending.
                handle_irq(&self.as_trap_frame(), PrivilegeLevel::User);
                crate::arch::irq::enable_local();

                if has_kernel_event() {
                    break ReturnReason::KernelEvent;
                }
                continue;
            }

            let esr = self.user_context.esr;
            let far = self.user_context.far;
            let exception = CpuException::new(esr, far);

            crate::arch::irq::enable_local();

            match exception {
                CpuException::Syscall => {
                    // Note: unlike aborts, `ELR_EL1` for an `SVC` already points
                    // to the instruction after the `SVC`, so no adjustment is
                    // needed here.
                    break ReturnReason::UserSyscall;
                }
                other => {
                    self.exception = Some(other);
                    break ReturnReason::UserException;
                }
            }
        }
    }

    fn as_trap_frame(&self) -> TrapFrame {
        TrapFrame {
            general: self.user_context.general,
            elr: self.user_context.elr,
            spsr: self.user_context.spsr,
            esr: self.user_context.esr,
        }
    }
}

impl UserContextApi for UserContext {
    fn trap_number(&self) -> usize {
        (self.user_context.esr >> 26) & 0b11_1111
    }

    fn trap_error_code(&self) -> usize {
        self.user_context.esr
    }

    fn instruction_pointer(&self) -> usize {
        self.user_context.elr
    }

    fn set_instruction_pointer(&mut self, ip: usize) {
        self.user_context.elr = ip;
    }

    fn stack_pointer(&self) -> usize {
        self.user_context.general.sp
    }

    fn set_stack_pointer(&mut self, sp: usize) {
        self.user_context.general.sp = sp;
    }
}

/// The FPU context of a user task (Advanced SIMD / floating-point state).
///
/// TODO: Save and restore the NEON `V0`-`V31`, `FPSR`, and `FPCR` registers in
/// assembly. Currently a placeholder that compiles and preserves a zeroed
/// buffer; user FP state is not yet context-switched.
#[repr(C, align(16))]
#[derive(Clone, Copy, Debug, Pod)]
pub struct FpuState {
    /// `V0`-`V31`, 128 bits each.
    v: [u128; 32],
    fpsr: u32,
    fpcr: u32,
    // Keeps the trailing fields a multiple of the 16-byte alignment so the
    // struct has no implicit padding (required by `Pod`).
    _reserved: [u32; 2],
}

impl Default for FpuState {
    fn default() -> Self {
        Self {
            v: [0; 32],
            fpsr: 0,
            fpcr: 0,
            _reserved: [0; 2],
        }
    }
}

/// The FPU context of a user task.
#[derive(Clone, Debug, Default)]
pub struct FpuContext {
    state: FpuState,
}

core::arch::global_asm!(include_str!("fpu.S"));

unsafe extern "C" {
    fn aarch64_save_fpu(state: *mut FpuState);
    fn aarch64_load_fpu(state: *const FpuState);
}

impl FpuContext {
    /// Creates a new FPU context.
    pub fn new() -> Self {
        Self::default()
    }

    /// Saves the CPU's current FPU context to this instance.
    pub fn save(&mut self) {
        // SAFETY: `state` is a valid, properly aligned `FpuState`.
        unsafe { aarch64_save_fpu(&mut self.state) };
    }

    /// Loads the CPU's FPU context from this instance.
    pub fn load(&self) {
        // SAFETY: `state` is a valid, properly aligned `FpuState`.
        unsafe { aarch64_load_fpu(&self.state) };
    }

    /// Returns the FPU context as a byte slice.
    pub fn as_bytes(&self) -> &[u8] {
        self.state.as_bytes()
    }

    /// Returns the FPU context as a mutable byte slice.
    pub fn as_bytes_mut(&mut self) -> &mut [u8] {
        self.state.as_mut_bytes()
    }
}
