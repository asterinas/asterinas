// SPDX-License-Identifier: MPL-2.0

//! CPU execution context control.

use alloc::boxed::Box;
use core::arch::x86_64::{_fxrstor64, _fxsave64, _xrstor64, _xsave64};

use bitflags::bitflags;
use cfg_if::cfg_if;
use log::debug;
use ostd_pod::Pod;
use spin::Once;
use x86::bits64::segmentation::wrfsbase;
use x86_64::registers::{
    control::{Cr0, Cr0Flags},
    rflags::RFlags,
    xcontrol::XCr0,
};

use crate::{
    arch::{
        trap::{RawUserContext, TrapFrame},
        CPU_FEATURES,
    },
    mm::Vaddr,
    task::scheduler,
    trap::call_irq_callback_functions,
    user::{ReturnReason, UserContextApi, UserContextApiInternal},
};

cfg_if! {
    if #[cfg(feature = "cvm_guest")] {
        mod tdx;

        use tdx::VirtualizationExceptionHandler;
    }
}

pub use x86::cpuid;

/// Userspace CPU context, including general-purpose registers and exception information.
#[derive(Clone, Default, Debug)]
#[repr(C)]
pub struct UserContext {
    user_context: RawUserContext,
    exception: Option<CpuException>,
}

/// General registers.
#[derive(Debug, Default, Clone, Copy, Eq, PartialEq)]
#[repr(C)]
#[expect(missing_docs)]
pub struct GeneralRegs {
    pub rax: usize,
    pub rbx: usize,
    pub rcx: usize,
    pub rdx: usize,
    pub rsi: usize,
    pub rdi: usize,
    pub rbp: usize,
    pub rsp: usize,
    pub r8: usize,
    pub r9: usize,
    pub r10: usize,
    pub r11: usize,
    pub r12: usize,
    pub r13: usize,
    pub r14: usize,
    pub r15: usize,
    pub rip: usize,
    pub rflags: usize,
    pub fsbase: usize,
    pub gsbase: usize,
}

/// Architectural CPU exceptions (x86-64 vectors 0-31).
///
/// For the authoritative specification of each vector, see the  
/// Intel® 64 and IA-32 Architectures Software Developer’s Manual,  
/// Volume 3 “System Programming Guide”, Chapter 6 “Interrupt and Exception
/// Handling”, in particular Section 6.15 “Exception and Interrupt
/// Reference”.
///
/// Every enum variant corresponds to one exception defined by the
/// Intel/AMD architecture.
/// Variants that naturally carry an error code (or other error information)
/// expose it through their associated data fields.
//
// TODO: Some exceptions (like `AlignmentCheck`) also push an
//       error code onto the stack, but that detail is not yet represented
//       in this type definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CpuException {
    ///  0 – #DE  Divide-by-zero error.
    DivisionError,
    ///  1 – #DB  Debug.
    Debug,
    ///  2 – NMI  Non-maskable interrupt.
    NonMaskableInterrupt,
    ///  3 – #BP  Breakpoint (INT3).
    BreakPoint,
    ///  4 – #OF  Overflow.
    Overflow,
    ///  5 – #BR  Bound-range exceeded.
    BoundRangeExceeded,
    ///  6 – #UD  Invalid or undefined opcode.
    InvalidOpcode,
    ///  7 – #NM  Device not available (FPU/MMX/SSE disabled).
    DeviceNotAvailable,
    ///  8 – #DF  Double fault (always pushes an error code of 0).
    DoubleFault,
    ///  9 – Coprocessor segment overrun (reserved on modern CPUs).
    CoprocessorSegmentOverrun,
    /// 10 – #TS  Invalid TSS.
    InvalidTss(SelectorErrorCode),
    /// 11 – #NP  Segment not present.
    SegmentNotPresent(SelectorErrorCode),
    /// 12 – #SS  Stack-segment fault.
    StackSegmentFault(SelectorErrorCode),
    /// 13 – #GP  General protection fault  
    GeneralProtectionFault(Option<SelectorErrorCode>),
    /// 14 – #PF  Page fault.
    PageFault(RawPageFaultInfo),
    // 15: Reserved
    /// 16 – #MF  x87 floating-point exception.
    X87FloatingPointException,
    /// 17 – #AC  Alignment check.  
    AlignmentCheck,
    /// 18 – #MC  Machine check.
    MachineCheck,
    /// 19 – #XM / #XF  SIMD/FPU floating-point exception.
    SIMDFloatingPointException,
    /// 20 – #VE  Virtualization exception.
    VirtualizationException,
    /// 21 – #CP  Control protection exception (CET).
    ControlProtectionException,
    // 22-27: Reserved
    /// 28 – #HV  Hypervisor injection exception.
    HypervisorInjectionException,
    /// 29 – #VC  VMM communication exception (SEV-ES GHCB).
    VMMCommunicationException,
    /// 30 – #SX  Security exception.
    SecurityException,
    // 31: Reserved
    /// Catch-all for reserved or undefined vector numbers.
    Reserved,
}

impl CpuException {
    pub(crate) fn new(trap_num: usize, error_code: usize) -> Option<Self> {
        let exception = match trap_num {
            0 => Self::DivisionError,
            1 => Self::Debug,
            2 => Self::NonMaskableInterrupt,
            3 => Self::BreakPoint,
            4 => Self::Overflow,
            5 => Self::BoundRangeExceeded,
            6 => Self::InvalidOpcode,
            7 => Self::DeviceNotAvailable,
            8 => {
                // A double fault will always generate an error code with a value of zero.
                debug_assert_eq!(error_code, 0);
                Self::DoubleFault
            }
            9 => Self::CoprocessorSegmentOverrun,
            10 => Self::InvalidTss(SelectorErrorCode(error_code)),
            11 => Self::SegmentNotPresent(SelectorErrorCode(error_code)),
            12 => Self::StackSegmentFault(SelectorErrorCode(error_code)),
            13 => {
                let error_code = if error_code == 0 {
                    None
                } else {
                    Some(SelectorErrorCode(error_code))
                };
                Self::GeneralProtectionFault(error_code)
            }
            14 => {
                let page_fault_addr = x86_64::registers::control::Cr2::read_raw() as usize;
                Self::PageFault(RawPageFaultInfo {
                    error_code: PageFaultErrorCode::from_bits(error_code).unwrap(),
                    addr: page_fault_addr,
                })
            }
            // Reserved 15
            16 => Self::X87FloatingPointException,
            17 => Self::AlignmentCheck,
            18 => Self::MachineCheck,
            19 => Self::SIMDFloatingPointException,
            20 => Self::VirtualizationException,
            21 => Self::ControlProtectionException,
            // Reserved 22-27
            28 => Self::HypervisorInjectionException,
            29 => Self::VMMCommunicationException,
            30 => Self::SecurityException,
            // Reserved 31
            15 | 22..=27 | 31 => Self::Reserved,
            _ => return None,
        };

        Some(exception)
    }

    const fn type_(&self) -> CpuExceptionType {
        match self {
            Self::Debug => CpuExceptionType::FaultOrTrap,
            Self::NonMaskableInterrupt => CpuExceptionType::Interrupt,
            Self::BreakPoint | Self::Overflow => CpuExceptionType::Trap,
            Self::DoubleFault | Self::MachineCheck => CpuExceptionType::Abort,
            Self::Reserved => CpuExceptionType::Reserved,
            _ => CpuExceptionType::Fault,
        }
    }

    pub(crate) const fn is_cpu_exception(trap_num: usize) -> bool {
        trap_num <= 31
    }
}

/// Selector error code.
///
/// Reference: <https://wiki.osdev.org/Exceptions#Selector_Error_Code>.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct SelectorErrorCode(usize);

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

        const SYSCALL_TRAPNUM: usize = 0x100;

        // Return when it is syscall or cpu exception type is Fault or Trap.
        loop {
            scheduler::might_preempt();
            self.user_context.run();

            let exception =
                CpuException::new(self.user_context.trap_num, self.user_context.error_code);
            match exception {
                #[cfg(feature = "cvm_guest")]
                Some(CpuException::VirtualizationException) => {
                    let ve_handler = VirtualizationExceptionHandler::new();
                    // Check out the doc of `VirtualizationExceptionHandler::new` to
                    // see why IRQs must enabled _after_ instantiating a `VirtualizationExceptionHandler`.
                    crate::arch::irq::enable_local();
                    ve_handler.handle(self);
                }
                Some(exception) if exception.type_().is_fault_or_trap() => {
                    crate::arch::irq::enable_local();
                    self.exception = Some(exception);
                    return ReturnReason::UserException;
                }
                Some(exception) => {
                    panic!(
                        "cannot handle user CPU exception: {:?}, trapframe: {:?}",
                        exception,
                        self.as_trap_frame()
                    );
                }
                None if self.user_context.trap_num == SYSCALL_TRAPNUM => {
                    crate::arch::irq::enable_local();
                    return ReturnReason::UserSyscall;
                }
                None => {
                    call_irq_callback_functions(
                        &self.as_trap_frame(),
                        self.as_trap_frame().trap_num,
                    );
                    crate::arch::irq::enable_local();
                }
            }

            if has_kernel_event() {
                break ReturnReason::KernelEvent;
            }
        }
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

impl CpuExceptionType {
    /// Returns whether this exception type is a fault or a trap.
    pub fn is_fault_or_trap(self) -> bool {
        match self {
            CpuExceptionType::Trap | CpuExceptionType::Fault | CpuExceptionType::FaultOrTrap => {
                true
            }
            CpuExceptionType::Abort | CpuExceptionType::Interrupt | CpuExceptionType::Reserved => {
                false
            }
        }
    }
}

/// Architecture-specific data reported with a page-fault exception.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawPageFaultInfo {
    /// The error code pushed by the CPU for this page fault.
    pub error_code: PageFaultErrorCode,
    /// The linear (virtual) address that triggered the fault (contents of CR2).
    pub addr: Vaddr,
}

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

/// The FPU context of user task.
///
/// This could be used for saving both legacy and modern state format.
#[derive(Debug)]
pub struct FpuContext {
    xsave_area: Box<XSaveArea>,
    area_size: usize,
}

impl FpuContext {
    /// Creates a new FPU context.
    pub fn new() -> Self {
        let mut area_size = size_of::<FxSaveArea>();
        if CPU_FEATURES.get().unwrap().has_xsave() {
            area_size = area_size.max(*XSAVE_AREA_SIZE.get().unwrap());
        }

        Self {
            xsave_area: Box::new(XSaveArea::new()),
            area_size,
        }
    }

    /// Saves CPU's current FPU context to this instance.
    pub fn save(&mut self) {
        let mem_addr = self.as_bytes_mut().as_mut_ptr();

        if CPU_FEATURES.get().unwrap().has_xsave() {
            unsafe { _xsave64(mem_addr, XFEATURE_MASK_USER_RESTORE) };
        } else {
            unsafe { _fxsave64(mem_addr) };
        }

        debug!("Save FPU context");
    }

    /// Loads CPU's FPU context from this instance.
    pub fn load(&mut self) {
        let mem_addr = self.as_bytes().as_ptr();

        if CPU_FEATURES.get().unwrap().has_xsave() {
            let rs_mask = XFEATURE_MASK_USER_RESTORE & XSTATE_MAX_FEATURES.get().unwrap();

            unsafe { _xrstor64(mem_addr, rs_mask) };
        } else {
            unsafe { _fxrstor64(mem_addr) };
        }

        debug!("Load FPU context");
    }

    /// Returns the FPU context as a byte slice.
    pub fn as_bytes(&self) -> &[u8] {
        &self.xsave_area.as_bytes()[..self.area_size]
    }

    /// Returns the FPU context as a mutable byte slice.
    pub fn as_bytes_mut(&mut self) -> &mut [u8] {
        &mut self.xsave_area.as_bytes_mut()[..self.area_size]
    }
}

impl Default for FpuContext {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for FpuContext {
    fn clone(&self) -> Self {
        let mut xsave_area = Box::new(XSaveArea::new());
        xsave_area.fxsave_area = self.xsave_area.fxsave_area;
        xsave_area.features = self.xsave_area.features;
        xsave_area.compaction = self.xsave_area.compaction;
        if self.area_size > size_of::<FxSaveArea>() {
            let len = self.area_size - size_of::<FxSaveArea>() - 64;
            xsave_area.extended_state_area[..len]
                .copy_from_slice(&self.xsave_area.extended_state_area[..len]);
        }

        Self {
            xsave_area,
            area_size: self.area_size,
        }
    }
}

/// The modern FPU context format (as saved and restored by the `XSAVE` and `XRSTOR` instructions).
#[repr(C)]
#[repr(align(64))]
#[derive(Clone, Copy, Debug, Pod)]
struct XSaveArea {
    fxsave_area: FxSaveArea,
    features: u64,
    compaction: u64,
    reserved: [u64; 6],
    extended_state_area: [u8; MAX_XSAVE_AREA_SIZE - size_of::<FxSaveArea>() - 64],
}

impl XSaveArea {
    fn new() -> Self {
        let features = if CPU_FEATURES.get().unwrap().has_xsave() {
            XCr0::read().bits() & XSTATE_MAX_FEATURES.get().unwrap()
        } else {
            0
        };

        let mut xsave_area = Self::new_zeroed();
        // Set the initial values for the FPU context. Refer to Intel SDM, Table 11-1:
        // "IA-32 and Intel® 64 Processor States Following Power-up, Reset, or INIT (Contd.)".
        xsave_area.fxsave_area.control = 0x037F;
        xsave_area.fxsave_area.tag = 0xFFFF;
        xsave_area.fxsave_area.mxcsr = 0x1F80;
        xsave_area.features = features;

        xsave_area
    }
}

/// The legacy SSE/MMX FPU context format (as saved and restored by the `FXSAVE` and `FXRSTOR` instructions).
#[repr(C)]
#[repr(align(16))]
#[derive(Clone, Copy, Debug, Pod)]
struct FxSaveArea {
    control: u16,         // x87 FPU Control Word
    status: u16,          // x87 FPU Status Word
    tag: u16,             // x87 FPU Tag Word
    op: u16,              // x87 FPU Last Instruction Opcode
    ip: u32,              // x87 FPU Instruction Pointer Offset
    cs: u32,              // x87 FPU Instruction Pointer Selector
    dp: u32,              // x87 FPU Instruction Operand (Data) Pointer Offset
    ds: u32,              // x87 FPU Instruction Operand (Data) Pointer Selector
    mxcsr: u32,           // MXCSR Register State
    mxcsr_mask: u32,      // MXCSR Mask
    st_space: [u32; 32], // x87 FPU or MMX technology registers (ST0-ST7 or MM0-MM7, 128 bits per field)
    xmm_space: [u32; 64], // XMM registers (XMM0-XMM15, 128 bits per field)
    padding: [u32; 12],  // Padding
    reserved: [u32; 12], // Software reserved
}

/// The XSTATE features (user & supervisor) supported by the processor.
static XSTATE_MAX_FEATURES: Once<u64> = Once::new();

/// Mask features which are restored when returning to user space.
///
/// X87 | SSE | AVX | OPMASK | ZMM_HI256 | HI16_ZMM
const XFEATURE_MASK_USER_RESTORE: u64 = 0b1110_0111;

/// The real size in bytes of the XSAVE area containing all states enabled by XCRO | IA32_XSS.
static XSAVE_AREA_SIZE: Once<usize> = Once::new();

/// The max size in bytes of the XSAVE area.
const MAX_XSAVE_AREA_SIZE: usize = 4096;

pub(in crate::arch) fn enable_essential_features() {
    XSTATE_MAX_FEATURES.call_once(|| {
        const XSTATE_CPUID: u32 = 0x0000000d;

        // Find user xstates supported by the processor.
        let res0 = cpuid::cpuid!(XSTATE_CPUID, 0);
        let mut features = res0.eax as u64 + ((res0.edx as u64) << 32);

        // Find supervisor xstates supported by the processor.
        let res1 = cpuid::cpuid!(XSTATE_CPUID, 1);
        features |= res1.ecx as u64 + ((res1.edx as u64) << 32);

        features
    });

    XSAVE_AREA_SIZE.call_once(|| {
        let cpuid = cpuid::CpuId::new();
        let size = cpuid
            .get_extended_state_info()
            .unwrap()
            .xsave_area_size_enabled_features() as usize;
        debug_assert!(size <= MAX_XSAVE_AREA_SIZE);
        size
    });

    if CPU_FEATURES.get().unwrap().has_fpu() {
        let mut cr0 = Cr0::read();
        cr0.remove(Cr0Flags::TASK_SWITCHED | Cr0Flags::EMULATE_COPROCESSOR);

        unsafe {
            Cr0::write(cr0);
            // Flush out any pending x87 state.
            core::arch::asm!("fninit");
        }
    }
}
