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

/// The FPU context of user task.
/// FIXME: Implement FPU context on ARM64 platforms.
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
