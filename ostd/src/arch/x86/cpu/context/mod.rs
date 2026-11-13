// SPDX-License-Identifier: MPL-2.0

//! CPU execution context control.

mod fpu;

use bitflags::bitflags;
pub use fpu::FpuContext;
pub(in crate::arch) use fpu::enable_essential_features;
use x86::bits64::segmentation::{rdfsbase, rdgsbase, swapgs, wrfsbase, wrgsbase};
use x86_64::registers::rflags::RFlags;

use crate::{
    arch::{
        irq::HwIrqLine,
        trap::{RawUserContext, TrapFrame},
    },
    cpu::PrivilegeLevel,
    irq::{DisabledLocalIrqGuard, call_irq_callback_functions},
    mm::Vaddr,
    user::{ReturnReason, UserContextApi, UserContextApiInternal, UserModeHooks},
};

cfg_select! {
    feature = "cvm_guest" => {
        mod tdx;

        use tdx::VirtualizationExceptionHandler;
    }
}

/// RFLAGS bits that non-privileged code may modify.
///
/// This is the union of the flags that Linux allows `ptrace` and `rt_sigreturn`
/// to modify. Because the set allowed by `rt_sigreturn` is a strict subset of
/// the `ptrace` set (it excludes `NT`), this is effectively the set of flags
/// that `ptrace` may modify.
/// Reference: <https://elixir.bootlin.com/linux/v7.1/source/arch/x86/include/asm/sighandling.h#L11-L14>.
/// Reference: <https://elixir.bootlin.com/linux/v7.1/source/arch/x86/kernel/ptrace.c#L158-L163>.
pub const USER_MODIFIABLE_RFLAGS: usize = (RFlags::CARRY_FLAG.bits()
    | RFlags::PARITY_FLAG.bits()
    | RFlags::AUXILIARY_CARRY_FLAG.bits()
    | RFlags::ZERO_FLAG.bits()
    | RFlags::SIGN_FLAG.bits()
    | RFlags::TRAP_FLAG.bits()
    | RFlags::DIRECTION_FLAG.bits()
    | RFlags::OVERFLOW_FLAG.bits()
    | RFlags::RESUME_FLAG.bits()
    | RFlags::ALIGNMENT_CHECK.bits()
    | RFlags::NESTED_TASK.bits()) as usize;

/// Userspace CPU context, including general-purpose registers and exception information.
#[repr(C)]
#[derive(Clone, Debug)]
pub struct UserContext {
    user_context: RawUserContext,
    exception: Option<CpuException>,
}

impl Default for UserContext {
    fn default() -> Self {
        // RFLAGS bit 1 is reserved, and Linux always sets it to 1 on x86.
        // Set the interrupt flag to enable the reception of external interrupts in user mode.
        // Set the ID flag to indicate that the CPU supports the CPUID instruction.
        const USER_MODE_INITIAL_RFLAGS: usize =
            (1 << 1) | (RFlags::INTERRUPT_FLAG.bits() | RFlags::ID.bits()) as usize;

        Self {
            user_context: RawUserContext {
                general: GeneralRegs::default(),
                rflags: USER_MODE_INITIAL_RFLAGS,
                trap_num: 0,
                error_code: 0,
            },
            exception: None,
        }
    }
}

/// General registers.
#[expect(missing_docs)]
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
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
}

/// The user-mode FS base register.
#[derive(Clone, Copy, Debug, Default)]
pub struct FsBase(usize);

impl FsBase {
    /// Creates a new `FsBase` with the given address.
    pub fn new(addr: usize) -> Self {
        Self(addr)
    }

    /// Returns the stored address.
    pub fn addr(&self) -> usize {
        self.0
    }

    /// Saves the current CPU FS base into this struct.
    pub fn save(&mut self) {
        // SAFETY: Reading the user FS base does not affect kernel code.
        self.0 = unsafe { rdfsbase() as usize };
    }

    /// Loads this struct's FS base onto the CPU.
    pub fn load(&self) {
        // SAFETY: Writing the user FS base does not affect kernel code.
        unsafe { wrfsbase(self.0 as u64) }
    }
}

/// The user-mode GS base register.
#[derive(Clone, Copy, Debug, Default)]
pub struct GsBase(usize);

impl GsBase {
    /// Creates a new `GsBase` with the given address.
    pub fn new(addr: usize) -> Self {
        Self(addr)
    }

    /// Returns the stored address.
    pub fn addr(&self) -> usize {
        self.0
    }

    /// Saves the current CPU GS base into this struct.
    pub fn save(&mut self, _guard: &DisabledLocalIrqGuard) {
        // SAFETY:
        // 1. In these steps, we have disabled the IRQ and are not using the kernel GS base.
        // 2. Reading the user GS base does not affect kernel code.
        unsafe {
            swapgs();
            self.0 = rdgsbase() as usize;
            swapgs();
        }
    }

    /// Loads this struct's GS base onto the CPU.
    pub fn load(&self, _guard: &DisabledLocalIrqGuard) {
        // SAFETY:
        // 1. In these steps, we have disabled the IRQ and are not using the kernel GS base.
        // 2. Writing the user GS base does not affect kernel code.
        unsafe {
            swapgs();
            wrgsbase(self.0 as u64);
            swapgs();
        }
    }
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
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
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
    pub(in crate::arch) fn new(trap_num: usize, error_code: usize) -> Option<Self> {
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

    pub(in crate::arch) const fn is_cpu_exception(trap_num: usize) -> bool {
        trap_num <= 31
    }
}

/// Selector error code.
///
/// Reference: <https://wiki.osdev.org/Exceptions#Selector_Error_Code>.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct SelectorErrorCode(usize);

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

    /// Gets the value of the RFLAGS register.
    pub fn rflags(&self) -> usize {
        self.user_context.rflags
    }

    /// Sets the value of the RFLAGS register.
    ///
    /// We only allow the setting or clearing of the bits in [`USER_MODIFIABLE_RFLAGS`].
    /// Any other bits will be ignored and will remain unchanged.
    pub fn set_rflags(&mut self, rflags: usize) {
        self.user_context.rflags = (self.user_context.rflags & !USER_MODIFIABLE_RFLAGS)
            | (rflags & USER_MODIFIABLE_RFLAGS);
    }
}

impl UserContextApiInternal for UserContext {
    fn execute<T: UserModeHooks>(&mut self, hooks: &T) -> ReturnReason {
        const SYSCALL_TRAPNUM: usize = 0x100;

        // Return when it is syscall or cpu exception type is Fault or Trap.
        loop {
            crate::task::scheduler::might_preempt();

            let guard = crate::irq::disable_local();
            hooks.pre_user_run(&guard);
            self.user_context.run(guard);

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
                        "Cannot handle user CPU exception: {:?}; trapframe: {:?}",
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
                        &HwIrqLine::new(self.as_trap_frame().trap_num as u8),
                        PrivilegeLevel::User,
                    );
                    crate::arch::irq::enable_local();
                }
            }

            if hooks.has_kernel_event() {
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
            r8: self.user_context.general.r8,
            r9: self.user_context.general.r9,
            r10: self.user_context.general.r10,
            r11: self.user_context.general.r11,
            r12: self.user_context.general.r12,
            r13: self.user_context.general.r13,
            r14: self.user_context.general.r14,
            r15: self.user_context.general.r15,
            trap_num: self.user_context.trap_num,
            error_code: self.user_context.error_code,
            rip: self.user_context.general.rip,
            cs: 0,
            rflags: self.user_context.rflags,
            rsp: self.user_context.general.rsp,
            ss: 0,
        }
    }
}

/// Types of CPU exceptions.
///
/// As defined by Intel, there are three types of x86-64 CPU exceptions:
///  - **Faults** can be corrected and the program may continue as if nothing happened.
///  - **Traps** are reported immediately after the execution of the trapping instruction.
///  - **Aborts** represent some unrecoverable errors.
///
/// However, there are some special vectors. Vector 1 can be either a fault or a trap, and vector 2
/// is an interrupt. Here, we also define `FaultOrTrap` and `Interrupt`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CpuExceptionType {
    /// Faults. They can be corrected and the program may continue as if nothing happened.
    Fault,
    /// Traps. They are reported immediately after the execution of the trapping instruction.
    Trap,
    /// Faults or traps.
    FaultOrTrap,
    /// Interrupts.
    Interrupt,
    /// Aborts. They represent some unrecoverable errors.
    Abort,
    /// Reserved for future use.
    Reserved,
}

impl CpuExceptionType {
    /// Returns whether this exception type is a fault or a trap.
    fn is_fault_or_trap(self) -> bool {
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
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RawPageFaultInfo {
    /// The error code pushed by the CPU for this page fault.
    pub error_code: PageFaultErrorCode,
    /// The linear (virtual) address that triggered the fault (contents of CR2).
    pub addr: Vaddr,
}

bitflags! {
    /// Page Fault error code. Following the Intel Architectures Software Developer's Manual Volume 3
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub struct PageFaultErrorCode: usize {
        /// 0 if no translation for the linear address.
        const PRESENT      = 1 << 0;
        /// 1 if the access was a write.
        const WRITE        = 1 << 1;
        /// 1 if the access was a user-mode access.
        const USER         = 1 << 2;
        /// 1 if there is no translation for the linear address
        /// because a reserved bit was set.
        const RESERVED     = 1 << 3;
        /// 1 if the access was an instruction fetch.
        const INSTRUCTION  = 1 << 4;
        /// 1 if the access was a data access to a linear address with a protection key for which
        /// the protection-key rights registers disallow access.
        const PROTECTION   = 1 << 5;
        /// 1 if the access was a shadow-stack access.
        const SHADOW_STACK = 1 << 6;
        /// 1 if there is no translation for the linear address using HLAT paging.
        const HLAT         = 1 << 7;
        /// 1 if the exception is unrelated to paging and resulted from violation of SGX-specific
        /// access-control requirements.
        const SGX          = 1 << 15;
    }
}

impl UserContextApi for UserContext {
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
    [rip, set_rip]
);
