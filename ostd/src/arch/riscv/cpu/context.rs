// SPDX-License-Identifier: MPL-2.0

//! CPU execution context control.

use core::fmt::Debug;

use riscv::register::scause::{Exception, Interrupt, Trap};

use crate::{
    arch::{
        timer::handle_timer_interrupt,
        trap::{RawUserContext, TrapFrame},
    },
    user::{ReturnReason, UserContextApi, UserContextApiInternal},
};

/// Userspace CPU context, including general-purpose registers and exception information.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct UserContext {
    user_context: RawUserContext,
    trap: Trap,
    cpu_exception_info: Option<CpuExceptionInfo>,
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

/// CPU exception information.
//
// TODO: Refactor the struct into an enum (similar to x86's `CpuException`).
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct CpuExceptionInfo {
    /// The type of the exception.
    pub code: Exception,
    /// The error code associated with the exception.
    pub page_fault_addr: usize,
    pub error_code: usize, // TODO
}

impl Default for UserContext {
    fn default() -> Self {
        UserContext {
            user_context: RawUserContext::default(),
            trap: Trap::Exception(Exception::Unknown),
            cpu_exception_info: None,
        }
    }
}

impl Default for CpuExceptionInfo {
    fn default() -> Self {
        CpuExceptionInfo {
            code: Exception::Unknown,
            page_fault_addr: 0,
            error_code: 0,
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
    pub fn general_regs(&self) -> &GeneralRegs {
        &self.user_context.general
    }

    /// Returns a mutable reference to the general registers
    pub fn general_regs_mut(&mut self) -> &mut GeneralRegs {
        &mut self.user_context.general
    }

    /// Returns the trap information.
    pub fn take_exception(&mut self) -> Option<CpuExceptionInfo> {
        self.cpu_exception_info.take()
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
        let ret = loop {
            self.user_context.run();
            match riscv::register::scause::read().cause() {
                Trap::Interrupt(Interrupt::SupervisorTimer) => {
                    handle_timer_interrupt();
                }
                Trap::Interrupt(_) => todo!(),
                Trap::Exception(Exception::UserEnvCall) => {
                    self.user_context.sepc += 4;
                    break ReturnReason::UserSyscall;
                }
                Trap::Exception(e) => {
                    let stval = riscv::register::stval::read();
                    log::trace!("Exception, scause: {e:?}, stval: {stval:#x?}");
                    self.cpu_exception_info = Some(CpuExceptionInfo {
                        code: e,
                        page_fault_addr: stval,
                        error_code: 0,
                    });
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

/// CPU exception.
pub type CpuException = Exception;

/// The FPU context of user task.
///
/// This could be used for saving both legacy and modern state format.
// FIXME: Implement FPU context on RISC-V platforms.
#[derive(Clone, Copy, Debug, Default)]
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
