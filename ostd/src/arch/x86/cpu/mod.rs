// SPDX-License-Identifier: MPL-2.0

//! CPU.

pub mod local;

use core::{
    arch::x86_64::{_fxrstor, _fxsave},
    fmt::Debug,
};

use bitflags::bitflags;
use cfg_if::cfg_if;
use log::debug;
use num_derive::FromPrimitive;
use num_traits::FromPrimitive;
use x86::bits64::segmentation::wrfsbase;
use x86_64::registers::rflags::RFlags;

pub use super::trap::GeneralRegs as RawGeneralRegs;
use super::trap::{TrapFrame, UserContext as RawUserContext};
use crate::{
    task::scheduler,
    trap::call_irq_callback_functions,
    user::{ReturnReason, UserContextApi, UserContextApiInternal},
};

cfg_if! {
    if #[cfg(feature = "cvm_guest")] {
        mod tdx;

        use tdx::handle_virtualization_exception;
    }
}

/// Cpu context, including both general-purpose registers and floating-point registers.
#[derive(Clone, Default, Copy, Debug)]
#[repr(C)]
pub struct UserContext {
    user_context: RawUserContext,
    fp_regs: FpRegs,
    cpu_exception_info: CpuExceptionInfo,
}

/// CPU exception information.
#[derive(Clone, Default, Copy, Debug)]
#[repr(C)]
pub struct CpuExceptionInfo {
    /// The ID of the exception.
    pub id: usize,
    /// The error code associated with the exception.
    pub error_code: usize,
    /// The virtual address where a page fault occurred.
    pub page_fault_addr: usize,
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

    /// Returns a reference to the floating point registers
    pub fn fp_regs(&self) -> &FpRegs {
        &self.fp_regs
    }

    /// Returns a mutable reference to the floating point registers
    pub fn fp_regs_mut(&mut self) -> &mut FpRegs {
        &mut self.fp_regs
    }

    /// Sets thread-local storage pointer.
    pub fn set_tls_pointer(&mut self, tls: usize) {
        self.set_fsbase(tls)
    }

    /// Gets thread-local storage pointer.
    pub fn tls_pointer(&self) -> usize {
        self.fsbase()
    }

    /// Activates thread-local storage pointer on the current CPU.
    ///
    /// # Safety
    ///
    /// The method by itself is safe because the value of the TLS register won't affect kernel code.
    /// But if the user relies on the TLS pointer, make sure that the pointer is correctly set when
    /// entering the user space.
    pub fn activate_tls_pointer(&self) {
        unsafe { wrfsbase(self.fsbase() as u64) }
    }
}

impl UserContextApiInternal for UserContext {
    fn execute<F>(&mut self, mut has_kernel_event: F) -> ReturnReason
    where
        F: FnMut() -> bool,
    {
        // set interrupt flag so that in user mode it can receive external interrupts
        // set ID flag which means cpu support CPUID instruction
        self.user_context.general.rflags |= (RFlags::INTERRUPT_FLAG | RFlags::ID).bits() as usize;

        const SYSCALL_TRAPNUM: u16 = 0x100;

        // return when it is syscall or cpu exception type is Fault or Trap.
        let return_reason = loop {
            scheduler::might_preempt();
            self.user_context.run();
            match CpuException::to_cpu_exception(self.user_context.trap_num as u16) {
                Some(exception) => {
                    #[cfg(feature = "cvm_guest")]
                    if exception == CpuException::VIRTUALIZATION_EXCEPTION {
                        handle_virtualization_exception(self);
                        continue;
                    }
                    match exception.typ() {
                        CpuExceptionType::FaultOrTrap
                        | CpuExceptionType::Trap
                        | CpuExceptionType::Fault => break ReturnReason::UserException,
                        _ => (),
                    }
                }
                None => {
                    if self.user_context.trap_num as u16 == SYSCALL_TRAPNUM {
                        break ReturnReason::UserSyscall;
                    }
                }
            };
            call_irq_callback_functions(&self.as_trap_frame(), self.as_trap_frame().trap_num);
            if has_kernel_event() {
                break ReturnReason::KernelEvent;
            }
        };

        crate::arch::irq::enable_local();
        if return_reason == ReturnReason::UserException {
            self.cpu_exception_info = CpuExceptionInfo {
                page_fault_addr: unsafe { x86::controlregs::cr2() },
                id: self.user_context.trap_num,
                error_code: self.user_context.error_code,
            };
        }

        return_reason
    }

    fn as_trap_frame(&self) -> TrapFrame {
        TrapFrame {
            rax: self.user_context.general.rax,
            rbx: self.user_context.general.rbx,
            rcx: self.user_context.general.rcx,
            rdx: self.user_context.general.rdx,
            rsi: self.user_context.general.rsi,
            rdi: self.user_context.general.rdi,
            rbp: self.user_context.general.rbp,
            rsp: self.user_context.general.rsp,
            r8: self.user_context.general.r8,
            r9: self.user_context.general.r9,
            r10: self.user_context.general.r10,
            r11: self.user_context.general.r11,
            r12: self.user_context.general.r12,
            r13: self.user_context.general.r13,
            r14: self.user_context.general.r14,
            r15: self.user_context.general.r15,
            _pad: 0,
            trap_num: self.user_context.trap_num,
            error_code: self.user_context.error_code,
            rip: self.user_context.general.rip,
            cs: 0,
            rflags: self.user_context.general.rflags,
        }
    }
}

/// As Osdev Wiki defines(<https://wiki.osdev.org/Exceptions>):
/// CPU exceptions are classified as:
///
/// Faults: These can be corrected and the program may continue as if nothing happened.
///
/// Traps: Traps are reported immediately after the execution of the trapping instruction.
///
/// Aborts: Some severe unrecoverable error.
///
/// But there exists some vector which are special. Vector 1 can be both fault or trap and vector 2 is interrupt.
/// So here we also define FaultOrTrap and Interrupt
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum CpuExceptionType {
    /// CPU faults. Faults can be corrected, and the program may continue as if nothing happened.
    Fault,
    /// CPU traps. Traps are reported immediately after the execution of the trapping instruction
    Trap,
    /// Faults or traps
    FaultOrTrap,
    /// CPU interrupts
    Interrupt,
    /// Some severe unrecoverable error
    Abort,
    /// Reserved for future use
    Reserved,
}

macro_rules! define_cpu_exception {
    ( $([ $name: ident = $exception_id:tt, $exception_type:tt]),* ) => {
        /// CPU exception.
        #[allow(non_camel_case_types)]
        #[derive(Debug, Copy, Clone, Eq, PartialEq, FromPrimitive)]
        pub enum CpuException {
            $(
                #[doc = concat!("The ", stringify!($name), " exception")]
                $name = $exception_id,
            )*
        }

        impl CpuException {
            /// The type of the CPU exception.
            pub fn typ(&self) -> CpuExceptionType {
                match self {
                    $( CpuException::$name => CpuExceptionType::$exception_type, )*
                }
            }
        }
    }
}

// We also defined the RESERVED Exception so that we can easily use the index of EXCEPTION_LIST to get the Exception
define_cpu_exception!(
    [DIVIDE_BY_ZERO = 0, Fault],
    [DEBUG = 1, FaultOrTrap],
    [NON_MASKABLE_INTERRUPT = 2, Interrupt],
    [BREAKPOINT = 3, Trap],
    [OVERFLOW = 4, Trap],
    [BOUND_RANGE_EXCEEDED = 5, Fault],
    [INVALID_OPCODE = 6, Fault],
    [DEVICE_NOT_AVAILABLE = 7, Fault],
    [DOUBLE_FAULT = 8, Abort],
    [COPROCESSOR_SEGMENT_OVERRUN = 9, Fault],
    [INVALID_TSS = 10, Fault],
    [SEGMENT_NOT_PRESENT = 11, Fault],
    [STACK_SEGMENT_FAULT = 12, Fault],
    [GENERAL_PROTECTION_FAULT = 13, Fault],
    [PAGE_FAULT = 14, Fault],
    [RESERVED_15 = 15, Reserved],
    [X87_FLOATING_POINT_EXCEPTION = 16, Fault],
    [ALIGNMENT_CHECK = 17, Fault],
    [MACHINE_CHECK = 18, Abort],
    [SIMD_FLOATING_POINT_EXCEPTION = 19, Fault],
    [VIRTUALIZATION_EXCEPTION = 20, Fault],
    [CONTROL_PROTECTION_EXCEPTION = 21, Fault],
    [RESERVED_22 = 22, Reserved],
    [RESERVED_23 = 23, Reserved],
    [RESERVED_24 = 24, Reserved],
    [RESERVED_25 = 25, Reserved],
    [RESERVED_26 = 26, Reserved],
    [RESERVED_27 = 27, Reserved],
    [HYPERVISOR_INJECTION_EXCEPTION = 28, Fault],
    [VMM_COMMUNICATION_EXCEPTION = 29, Fault],
    [SECURITY_EXCEPTION = 30, Fault],
    [RESERVED_31 = 31, Reserved]
);

bitflags! {
    /// Page Fault error code. Following the Intel Architectures Software Developer's Manual Volume 3
    pub struct PageFaultErrorCode : usize{
        /// 0 if no translation for the linear address.
        const PRESENT       = 1 << 0;
        /// 1 if the access was a write.
        const WRITE         = 1 << 1;
        /// 1 if the access was a user-mode access.
        const USER          = 1 << 2;
        /// 1 if there is no translation for the linear address
        /// because a reserved bit was set.
        const RESERVED      = 1 << 3;
        /// 1 if the access was an instruction fetch.
        const INSTRUCTION   = 1 << 4;
        /// 1 if the access was a data access to a linear address with a protection key for which
        /// the protection-key rights registers disallow access.
        const PROTECTION    = 1 << 5;
        /// 1 if the access was a shadow-stack access.
        const SHADOW_STACK  = 1 << 6;
        /// 1 if there is no translation for the linear address using HLAT paging.
        const HLAT          = 1 << 7;
        /// 1 if the exception is unrelated to paging and resulted from violation of SGX-specific
        /// access-control requirements.
        const SGX           = 1 << 15;
    }
}

impl CpuException {
    /// Checks if the given `trap_num` is a valid CPU exception.
    pub fn is_cpu_exception(trap_num: u16) -> bool {
        Self::to_cpu_exception(trap_num).is_some()
    }

    /// Maps a `trap_num` to its corresponding CPU exception.
    pub fn to_cpu_exception(trap_num: u16) -> Option<CpuException> {
        FromPrimitive::from_u16(trap_num)
    }
}

impl CpuExceptionInfo {
    /// Get corresponding CPU exception
    pub fn cpu_exception(&self) -> CpuException {
        CpuException::to_cpu_exception(self.id as u16).unwrap()
    }
}

impl UserContextApi for UserContext {
    fn trap_number(&self) -> usize {
        self.user_context.trap_num
    }

    fn trap_error_code(&self) -> usize {
        self.user_context.error_code
    }

    fn set_instruction_pointer(&mut self, ip: usize) {
        self.set_rip(ip);
    }

    fn set_stack_pointer(&mut self, sp: usize) {
        self.set_rsp(sp)
    }

    fn stack_pointer(&self) -> usize {
        self.rsp()
    }

    fn instruction_pointer(&self) -> usize {
        self.rip()
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

                #[doc = concat!("Sets the value of ", stringify!(field))]
                #[inline(always)]
                pub fn $setter_name(&mut self, $field: usize) {
                    self.user_context.general.$field = $field;
                }
            )*
        }
    };
}

cpu_context_impl_getter_setter!(
    [rax, set_rax],
    [rbx, set_rbx],
    [rcx, set_rcx],
    [rdx, set_rdx],
    [rsi, set_rsi],
    [rdi, set_rdi],
    [rbp, set_rbp],
    [rsp, set_rsp],
    [r8, set_r8],
    [r9, set_r9],
    [r10, set_r10],
    [r11, set_r11],
    [r12, set_r12],
    [r13, set_r13],
    [r14, set_r14],
    [r15, set_r15],
    [rip, set_rip],
    [rflags, set_rflags],
    [fsbase, set_fsbase],
    [gsbase, set_gsbase]
);

/// The floating-point state of CPU.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct FpRegs {
    buf: FxsaveArea,
    is_valid: bool,
}

impl FpRegs {
    /// Creates a new instance.
    ///
    /// Note that a newly-created instance's floating point state is not
    /// initialized, thus considered invalid (i.e., `self.is_valid() == false`).
    pub fn new() -> Self {
        // The buffer address requires 16bytes alignment.
        Self {
            buf: FxsaveArea { data: [0; 512] },
            is_valid: false,
        }
    }

    /// Save CPU's current floating pointer states into this instance.
    pub fn save(&mut self) {
        debug!("save fpregs");
        debug!("write addr = 0x{:x}", (&mut self.buf) as *mut _ as usize);
        let layout = alloc::alloc::Layout::for_value(&self.buf);
        debug!("layout: {:?}", layout);
        let ptr = unsafe { alloc::alloc::alloc(layout) } as usize;
        debug!("ptr = 0x{:x}", ptr);
        unsafe {
            _fxsave(self.buf.data.as_mut_ptr());
        }
        debug!("save fpregs success");
        self.is_valid = true;
    }

    /// Saves the floating state given by a slice of u8.
    ///
    /// After calling this method, the state of the instance will be considered valid.
    ///
    /// # Safety
    ///
    /// It is the caller's responsibility to ensure that the source slice contains
    /// data that is in xsave/xrstor format. The slice must have a length of 512 bytes.
    pub unsafe fn save_from_slice(&mut self, src: &[u8]) {
        self.buf.data.copy_from_slice(src);
        self.is_valid = true;
    }

    /// Returns whether the instance can contains data in valid xsave/xrstor format.
    pub fn is_valid(&self) -> bool {
        self.is_valid
    }

    /// Clears the state of the instance.
    ///
    /// This method does not reset the underlying buffer that contains the floating
    /// point state; it only marks the buffer __invalid__.
    pub fn clear(&mut self) {
        self.is_valid = false;
    }

    /// Restores CPU's CPU floating pointer states from this instance.
    ///
    /// # Panics
    ///
    /// If the current state is invalid, the method will panic.
    pub fn restore(&self) {
        debug!("restore fpregs");
        assert!(self.is_valid);
        unsafe { _fxrstor(self.buf.data.as_ptr()) };
        debug!("restore fpregs success");
    }

    /// Returns the floating point state as a slice.
    ///
    /// Note that the slice may contain garbage if `self.is_valid() == false`.
    pub fn as_slice(&self) -> &[u8] {
        &self.buf.data
    }
}

impl Default for FpRegs {
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C, align(16))]
#[derive(Debug, Clone, Copy)]
struct FxsaveArea {
    data: [u8; 512], // 512 bytes
}
