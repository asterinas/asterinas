// SPDX-License-Identifier: MPL-2.0

//! CPU

pub mod local;

use core::fmt::Debug;

use riscv::register::scause::{Exception, Trap};

pub use super::trap::GeneralRegs as RawGeneralRegs;
use super::trap::{TrapFrame, UserContext as RawUserContext};
use crate::user::{ReturnReason, UserContextApi, UserContextApiInternal};

/// Cpu context, including both general-purpose registers and FPU state.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct UserContext {
    user_context: RawUserContext,
    trap: Trap,
    fpu_state: (), // TODO
    cpu_exception_info: CpuExceptionInfo,
}

/// CPU exception information.
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
            fpu_state: (),
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
    pub fn fpu_state(&self) -> &() {
        &self.fpu_state
    }

    /// Returns a mutable reference to the FPU state.
    pub fn fpu_state_mut(&mut self) -> &mut () {
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
}

impl UserContextApiInternal for UserContext {
    fn execute<F>(&mut self, mut has_kernel_event: F) -> ReturnReason
    where
        F: FnMut() -> bool,
    {
        let ret = loop {
            self.user_context.run();
            match riscv::register::scause::read().cause() {
                Trap::Interrupt(_) => todo!(),
                Trap::Exception(Exception::UserEnvCall) => {
                    self.user_context.sepc += 4;
                    break ReturnReason::UserSyscall;
                }
                Trap::Exception(e) => {
                    let stval = riscv::register::stval::read();
                    log::trace!("Exception, scause: {e:?}, stval: {stval:#x?}");
                    self.cpu_exception_info = CpuExceptionInfo {
                        code: e,
                        page_fault_addr: stval,
                        error_code: 0,
                    };
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
