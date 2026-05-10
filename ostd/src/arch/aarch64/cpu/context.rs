// SPDX-License-Identifier: MPL-2.0

//! CPU execution context control.

use crate::{
    arch::trap::{RawUserContext, TrapFrame},
    user::{ReturnReason, UserContextApi, UserContextApiInternal},
};

/// General registers.
#[expect(missing_docs)]
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct GeneralRegs {
    pub x0: usize,
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
    pub x29: usize, // Frame pointer
    pub x30: usize, // Link register
    pub sp: usize,
}

/// ARM64 CPU exceptions.
#[derive(Clone, Copy, Debug)]
pub enum CpuException {
    /// Instruction page fault (Instruction Abort from lower EL).
    InstructionPageFault(usize),
    /// Load page fault (Data Abort, WnR=0, from lower EL).
    LoadPageFault(usize),
    /// Store page fault (Data Abort, WnR=1, from lower EL).
    StorePageFault(usize),
    /// Unknown trap.
    Unknown,
}

/// Userspace CPU context, including general-purpose registers and exception information.
#[repr(C)]
#[derive(Clone, Debug, Default)]
pub struct UserContext {
    user_context: RawUserContext,
    exception: Option<CpuException>,
}

impl UserContext {
    /// Returns the saved processor state (SPSR_EL1) for use in signal context save/restore.
    pub fn spsr_el1(&self) -> usize {
        self.user_context.spsr_el1
    }

    /// Sets the saved processor state (SPSR_EL1).
    pub fn set_spsr_el1(&mut self, pstate: usize) {
        self.user_context.spsr_el1 = pstate;
    }

    /// Returns a reference to the general registers.
    pub fn general_regs(&self) -> &GeneralRegs {
        &self.user_context.general
    }

    /// Returns a mutable reference to the general registers.
    pub fn general_regs_mut(&mut self) -> &mut GeneralRegs {
        &mut self.user_context.general
    }

    /// Returns the trap information.
    pub fn take_exception(&mut self) -> Option<CpuException> {
        self.exception.take()
    }

    /// Sets the thread-local storage pointer.
    pub fn set_tls_pointer(&mut self, tls: usize) {
        self.user_context.tls_pointer = tls;
    }

    /// Gets the thread-local storage pointer.
    pub fn tls_pointer(&self) -> usize {
        self.user_context.tls_pointer
    }

    /// Activates the thread-local storage pointer for the current task.
    ///
    /// On AArch64, this writes the TPIDR_EL0 system register directly.
    /// Unlike x86 (which must write FS base via MSR), this takes effect immediately
    /// and user-space can use the TLS pointer right away.
    ///
    /// Note: This is called at task entry to set the initial TLS value from
    /// `UserContext`. Subsequent context switches use `ThreadLocal.tls_value`
    /// instead — see `kernel/src/process/posix_thread/thread_local.rs`.
    pub fn activate_tls_pointer(&self) {
        // SAFETY: Writing TPIDR_EL0 only affects the current thread's user-space
        // TLS pointer. It will be restored by the scheduler on next context switch.
        unsafe { core::arch::asm!("msr tpidr_el0, {0}", in(reg) self.user_context.tls_pointer) };
    }
}

impl UserContextApiInternal for UserContext {
    fn execute<F>(&mut self, mut has_kernel_event: F) -> ReturnReason
    where
        F: FnMut() -> bool,
    {
        // Clear IRQ mask bit in SPSR_EL1 to enable interrupts in user mode.
        // SPSR_EL1 DAIF bits: [9:6] = D, A, I, F. Bit 7 (I) masks IRQs.
        // Clearing bit 7 allows IRQs (timer, UART) to fire in EL0.
        self.user_context.spsr_el1 &= !(1 << 7);

        // Currently all traps that return from run() require breaking out of the
        // loop (SVC, page fault, unknown exception), so this loop never actually
        // loops. The loop structure is kept for consistency with x86/riscv and
        // to accommodate future inline-handled traps that would continue looping.
        #[expect(clippy::never_loop)]
        loop {
            crate::task::scheduler::might_preempt();
            self.user_context.run();

            // run() returns only for traps that need kernel processing
            // (syscalls, unhandled exceptions). Inline-handled traps
            // (mapped page faults, IRQs) never return from run().

            // Check for pending kernel events (e.g., signals) before
            // dispatching the trap, matching x86/riscv behavior.
            if has_kernel_event() {
                break ReturnReason::KernelEvent;
            }

            // Read ESR_EL1 to determine the return reason. The exception
            // class was preserved from the original exception because
            // DAIF is masked during the run_user_done return path.
            let esr: u64;
            // SAFETY: Reading ESR_EL1 is always safe.
            unsafe { core::arch::asm!("mrs {0}, esr_el1", out(reg) esr) };
            let ec = (esr >> 26) as u8 & 0x3f;

            match ec {
                // EC = 0x15: SVC (system call)
                0x15 => {
                    crate::arch::irq::enable_local();
                    break ReturnReason::UserSyscall;
                }
                // EC = 0x20/0x24: User page faults
                0x20 | 0x24 => {
                    let far: u64;
                    // SAFETY: Reading FAR_EL1 is always safe.
                    unsafe { core::arch::asm!("mrs {0}, far_el1", out(reg) far) };
                    let exception = match ec {
                        0x20 => CpuException::InstructionPageFault(far as usize),
                        _ => {
                            if esr & (1 << 6) != 0 {
                                CpuException::StorePageFault(far as usize)
                            } else {
                                CpuException::LoadPageFault(far as usize)
                            }
                        }
                    };
                    self.exception = Some(exception);
                    crate::arch::irq::enable_local();
                    break ReturnReason::UserException;
                }
                // Other: treat as unknown exception
                _ => {
                    self.exception = Some(CpuException::Unknown);
                    crate::arch::irq::enable_local();
                    break ReturnReason::UserException;
                }
            }
        }
    }

    fn as_trap_frame(&self) -> TrapFrame {
        TrapFrame {
            general: self.user_context.general,
            spsr_el1: self.user_context.spsr_el1,
            elr_el1: self.user_context.elr_el1,
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
        self.user_context.elr_el1
    }

    fn set_instruction_pointer(&mut self, ip: usize) {
        self.user_context.elr_el1 = ip;
    }

    fn stack_pointer(&self) -> usize {
        self.user_context.general.sp
    }

    fn set_stack_pointer(&mut self, sp: usize) {
        self.user_context.general.sp = sp;
    }
}

macro_rules! cpu_context_impl_getter_setter {
    ( $( [ $field: ident, $setter_name: ident] ),* ) => {
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
    [x30, set_ra],
    [sp, set_sp]
);

/// The FPU/SIMD context of a user task.
///
/// ARM64 FPU state:
/// - 32 128-bit SIMD/FP registers (Q0-Q31 / V0-V31): 512 bytes
/// - FPSR (Floating-point Status Register): 4 bytes
/// - FPCR (Floating-point Control Register): 4 bytes
///   Total: 520 bytes.
#[derive(Clone, Debug, Default)]
#[repr(C, align(16))]
pub struct FpuContext {
    /// SIMD/FP registers Q0-Q31.
    pub q: [u128; 32],
    /// Floating-point Status Register.
    pub fpsr: u32,
    /// Floating-point Control Register.
    pub fpcr: u32,
}

impl FpuContext {
    /// Creates a new FPU context with default (zero) state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Saves CPU's current FPU context to this instance.
    pub fn save(&mut self) {
        // SAFETY: The pointer to self is valid and the STP instructions
        // store 16-byte SIMD register values to aligned memory.
        unsafe { fpu_save(self) };
    }

    /// Loads CPU's FPU context from this instance.
    pub fn load(&mut self) {
        // SAFETY: The pointer to self is valid and the LDP instructions
        // load 16-byte SIMD register values from aligned memory.
        unsafe { fpu_load(self) };
    }

    /// Returns the FPU context as a byte slice.
    pub fn as_bytes(&self) -> &[u8] {
        // SAFETY: FpuContext is repr(C) with known layout.
        unsafe { core::slice::from_raw_parts(self as *const Self as *const u8, size_of::<Self>()) }
    }

    /// Returns the FPU context as a mutable byte slice.
    pub fn as_bytes_mut(&mut self) -> &mut [u8] {
        // SAFETY: FpuContext is repr(C) with known layout.
        unsafe { core::slice::from_raw_parts_mut(self as *mut Self as *mut u8, size_of::<Self>()) }
    }
}

// SAFETY: The assembly functions save/restore Q0-Q31 + FPSR + FPCR.
unsafe extern "C" {
    fn fpu_save(ctx: *mut FpuContext);
    fn fpu_load(ctx: *mut FpuContext);
}

core::arch::global_asm!(
    r#"
.section .text, "ax", @progbits
.global fpu_save
fpu_save:
    .arch armv8-a+fp+simd
    stp     q0, q1,   [x0, #0x00]
    stp     q2, q3,   [x0, #0x20]
    stp     q4, q5,   [x0, #0x40]
    stp     q6, q7,   [x0, #0x60]
    stp     q8, q9,   [x0, #0x80]
    stp     q10, q11, [x0, #0xA0]
    stp     q12, q13, [x0, #0xC0]
    stp     q14, q15, [x0, #0xE0]
    stp     q16, q17, [x0, #0x100]
    stp     q18, q19, [x0, #0x120]
    stp     q20, q21, [x0, #0x140]
    stp     q22, q23, [x0, #0x160]
    stp     q24, q25, [x0, #0x180]
    stp     q26, q27, [x0, #0x1A0]
    stp     q28, q29, [x0, #0x1C0]
    stp     q30, q31, [x0, #0x1E0]
    mrs     x1, S3_3_C4_C4_1  // FPSR
    str     w1, [x0, #0x200]
    mrs     x1, S3_3_C4_C4_0  // FPCR
    str     w1, [x0, #0x204]
    ret

.global fpu_load
fpu_load:
    .arch armv8-a+fp+simd
    ldp     q0, q1,   [x0, #0x00]
    ldp     q2, q3,   [x0, #0x20]
    ldp     q4, q5,   [x0, #0x40]
    ldp     q6, q7,   [x0, #0x60]
    ldp     q8, q9,   [x0, #0x80]
    ldp     q10, q11, [x0, #0xA0]
    ldp     q12, q13, [x0, #0xC0]
    ldp     q14, q15, [x0, #0xE0]
    ldp     q16, q17, [x0, #0x100]
    ldp     q18, q19, [x0, #0x120]
    ldp     q20, q21, [x0, #0x140]
    ldp     q22, q23, [x0, #0x160]
    ldp     q24, q25, [x0, #0x180]
    ldp     q26, q27, [x0, #0x1A0]
    ldp     q28, q29, [x0, #0x1C0]
    ldp     q30, q31, [x0, #0x1E0]
    ldr     w1, [x0, #0x200]
    msr     S3_3_C4_C4_1, x1  // FPSR
    ldr     w1, [x0, #0x204]
    msr     S3_3_C4_C4_0, x1  // FPCR
    ret
"#,
);
