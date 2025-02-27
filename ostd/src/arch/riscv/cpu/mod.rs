// SPDX-License-Identifier: MPL-2.0

//! CPU

pub mod local;

use core::fmt::Debug;

use riscv::register::{
    scause::{Exception, Interrupt, Trap},
    sstatus,
};

pub use super::trap::GeneralRegs as RawGeneralRegs;
use super::{
    irq::TIMER_IRQ_LINE,
    trap::{handle_external_interrupts, TrapFrame, UserContext as RawUserContext},
};
use crate::{
    prelude::*,
    task::scheduler,
    trap::call_irq_callback_functions,
    user::{ReturnReason, UserContextApi, UserContextApiInternal},
};

/// Cpu context, including both general-purpose registers and FPU state.
#[derive(Clone, Debug)]
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
    /// The error code associated with the exception.
    pub page_fault_addr: usize,
    pub error_code: usize, // TODO
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
}

impl UserContextApiInternal for UserContext {
    fn execute<F>(&mut self, mut has_kernel_event: F) -> ReturnReason
    where
        F: FnMut() -> bool,
    {
        let ret = loop {
            scheduler::might_preempt();

            const FS_MASK: usize = 0b11 << 13;
            self.user_context.sstatus =
                (self.user_context.sstatus & !FS_MASK) | ((self.fpu_state.fs as usize) << 13);
            self.user_context.run();
            self.fpu_state.fs = bits_to_fs((self.user_context.sstatus >> 13) & 0b11);

            match riscv::register::scause::read().cause() {
                Trap::Interrupt(Interrupt::SupervisorTimer) => {
                    call_irq_callback_functions(&self.as_trap_frame(), TIMER_IRQ_LINE)
                }
                Trap::Interrupt(Interrupt::SupervisorExternal) => {
                    handle_external_interrupts(&self.as_trap_frame())
                }
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
            crate::arch::irq::enable_local();

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

core::arch::global_asm!(include_str!("fpu.S"));

extern "C" {
    fn fstate_save(buf: *mut FpRegs);
    fn fstate_restore(buf: &FpRegs);
}

/// The FPU state of user task.
#[derive(Debug, Clone)]
pub struct FpuState {
    fs: sstatus::FS,
    floating_state: Box<FpRegs>,
}

#[derive(Debug, Clone, Default)]
#[repr(C)]
pub struct FpRegs {
    fregs: [f64; 32],
    fcsr: u64, // Floating-Point Control and Status Register
}

impl FpuState {
    /// Save CPU's current FPU state into this instance.
    pub fn save(&self) {
        if self.fs == sstatus::FS::Dirty {
            self.floating_state.save();
        }
        log::trace!("FPU state saved");
    }

    /// Restores CPU's FPU state from this instance.
    pub fn restore(&self) {
        self.floating_state.restore();

        // TODO: uncomment this line to reduce context-swap overhead once `&mut self` is allowed
        // self.fs = sstatus::FS::Clean;

        log::trace!("FPU state restored");
    }
}

impl Default for FpuState {
    fn default() -> Self {
        FpuState {
            fs: sstatus::FS::Initial,
            floating_state: Box::new(FpRegs::default()),
        }
    }
}

impl FpRegs {
    /// Save CPU's floating point registers into this instance.
    pub fn save(&self) {
        // FIXME: I don't know why `FpuState::save` is called without `&mut`.
        unsafe {
            fstate_save(self as *const FpRegs as *mut FpRegs);
        }
    }

    /// Restores CPU's floating point registers from this instance.
    pub fn restore(&self) {
        unsafe {
            fstate_restore(self);
        }
    }
}

fn bits_to_fs(bits: usize) -> sstatus::FS {
    match bits {
        0 => sstatus::FS::Off,
        1 => sstatus::FS::Initial,
        2 => sstatus::FS::Clean,
        3 => sstatus::FS::Dirty,
        _ => unreachable!(),
    }
}

/// CPU exception.
pub type CpuException = Exception;
