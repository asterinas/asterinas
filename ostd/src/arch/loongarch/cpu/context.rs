// SPDX-License-Identifier: MPL-2.0

//! CPU execution context control.

use alloc::boxed::Box;
use core::{arch::asm, fmt::Debug};

use loongArch64::register::estat::{Exception, Interrupt, Trap};
use ostd_pod::IntoBytes;

use crate::{
    arch::{
        irq::HwIrqLine,
        mm::tlb_flush_addr,
        trap::{RawUserContext, TrapFrame},
    },
    cpu::PrivilegeLevel,
    irq::call_irq_callback_functions,
    user::{ReturnReason, UserContextApi, UserContextApiInternal, UserModeHooks},
};

/// General registers.
#[expect(missing_docs)]
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct GeneralRegs {
    pub zero: usize,
    pub ra: usize,
    pub tp: usize,
    pub sp: usize,
    pub a0: usize,
    pub a1: usize,
    pub a2: usize,
    pub a3: usize,
    pub a4: usize,
    pub a5: usize,
    pub a6: usize,
    pub a7: usize,
    pub t0: usize,
    pub t1: usize,
    pub t2: usize,
    pub t3: usize,
    pub t4: usize,
    pub t5: usize,
    pub t6: usize,
    pub t7: usize,
    pub t8: usize,
    pub r21: usize,
    pub fp: usize,
    pub s0: usize,
    pub s1: usize,
    pub s2: usize,
    pub s3: usize,
    pub s4: usize,
    pub s5: usize,
    pub s6: usize,
    pub s7: usize,
    pub s8: usize,
}

/// LoongArch CPU exceptions.
///
/// Every enum variant corresponds to one exception defined by the LoongArch
/// architecture. Variants that naturally carry a faulting virtual address
/// (in the `badv` register) expose it through their associated data fields.
#[derive(Clone, Copy, Debug)]
pub enum CpuException {
    /// Load page fault exception.
    LoadPageFault(FaultAddress),
    /// Store page fault exception.
    StorePageFault(FaultAddress),
    /// Fetch page fault exception.
    FetchPageFault(FaultAddress),
    /// Page modify fault exception.
    PageModifyFault(FaultAddress),
    /// Page non-readable fault exception.
    PageNonReadableFault(FaultAddress),
    /// Page non-executable fault exception.
    PageNonExecutableFault(FaultAddress),
    /// Page privilege illegal exception.
    PagePrivilegeIllegal(FaultAddress),
    /// Fetch instruction address error exception.
    FetchInstructionAddressError,
    /// Memory access address error exception.
    MemoryAccessAddressError,
    /// Address not aligned exception.
    AddressNotAligned,
    /// Bounds check fault exception.
    BoundsCheckFault,
    /// System call exception.
    Syscall,
    /// Breakpoint exception.
    Breakpoint,
    /// Instruction does not exist exception.
    InstructionNotExist,
    /// Instruction privilege illegal exception.
    InstructionPrivilegeIllegal,
    /// Floating-point unit unavailable exception.
    FloatingPointUnavailable,
    /// Floating-point arithmetic exception.
    FloatingPoint {
        /// The floating-point control and status register at the fault.
        fcsr: u32,
    },
}

const ECODE_FPE: usize = 0x12;

/// Faulting virtual address of address-related exceptions.
pub type FaultAddress = usize;

impl CpuException {
    pub(in crate::arch) fn new(raw_exception: Exception, badv: usize) -> Self {
        match raw_exception {
            Exception::LoadPageFault => Self::LoadPageFault(badv),
            Exception::StorePageFault => Self::StorePageFault(badv),
            Exception::FetchPageFault => Self::FetchPageFault(badv),
            Exception::PageModifyFault => Self::PageModifyFault(badv),
            Exception::PageNonReadableFault => Self::PageNonReadableFault(badv),
            Exception::PageNonExecutableFault => Self::PageNonExecutableFault(badv),
            Exception::PagePrivilegeIllegal => Self::PagePrivilegeIllegal(badv),
            Exception::FetchInstructionAddressError => Self::FetchInstructionAddressError,
            Exception::MemoryAccessAddressError => Self::MemoryAccessAddressError,
            Exception::AddressNotAligned => Self::AddressNotAligned,
            Exception::BoundsCheckFault => Self::BoundsCheckFault,
            Exception::Syscall => Self::Syscall,
            Exception::Breakpoint => Self::Breakpoint,
            Exception::InstructionNotExist => Self::InstructionNotExist,
            Exception::InstructionPrivilegeIllegal => Self::InstructionPrivilegeIllegal,
            Exception::FloatingPointUnavailable => Self::FloatingPointUnavailable,
            Exception::TLBRFill => unreachable!(),
        }
    }
}

/// Userspace CPU context, including general-purpose registers and exception information.
#[repr(C)]
#[derive(Clone, Debug)]
pub struct UserContext {
    user_context: RawUserContext,
    trap: Trap,
    exception: Option<CpuException>,
}

impl Default for UserContext {
    fn default() -> Self {
        UserContext {
            user_context: RawUserContext::default(),
            trap: Trap::Unknown,
            exception: None,
        }
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
        self.tp()
    }

    /// Sets the value of the thread-local storage pointer.
    pub fn set_tls_pointer(&mut self, tls: usize) {
        self.set_tp(tls)
    }
}

impl UserContextApiInternal for UserContext {
    fn execute<T: UserModeHooks>(&mut self, hooks: &T) -> ReturnReason {
        loop {
            crate::task::scheduler::might_preempt();

            let irq_guard = crate::irq::disable_local();
            hooks.pre_user_run(&irq_guard);
            self.user_context.run(&irq_guard);

            let estat = loongArch64::register::estat::read();
            let cause = estat.cause();
            let badv = loongArch64::register::badv::read().raw();
            let badi = loongArch64::register::badi::read().raw();
            let era = loongArch64::register::era::read().raw();

            match cause {
                Trap::Exception(exception) => match exception {
                    Exception::Syscall => {
                        self.user_context.era += 4;
                        break ReturnReason::UserSyscall;
                    }
                    Exception::LoadPageFault
                    | Exception::StorePageFault
                    | Exception::FetchPageFault
                    | Exception::PageModifyFault
                    | Exception::PageNonReadableFault
                    | Exception::PageNonExecutableFault
                    | Exception::PagePrivilegeIllegal => {
                        // Handle page fault
                        tlb_flush_addr(badv);
                        self.exception = Some(CpuException::new(exception, badv));
                        break ReturnReason::UserException;
                    }
                    Exception::FetchInstructionAddressError
                    | Exception::MemoryAccessAddressError
                    | Exception::AddressNotAligned
                    | Exception::BoundsCheckFault
                    | Exception::Breakpoint
                    | Exception::InstructionNotExist
                    | Exception::InstructionPrivilegeIllegal
                    | Exception::FloatingPointUnavailable => {
                        // Handle other exceptions
                        self.exception = Some(CpuException::new(exception, badv));
                        crate::debug!(
                            "Exception {exception:?} occurred, badv: {badv:#x?}, badi: {badi:#x?}, era: {era:#x?}"
                        );
                        break ReturnReason::UserException;
                    }
                    Exception::TLBRFill => unreachable!(),
                },
                Trap::Interrupt(interrupt) => match interrupt {
                    Interrupt::SWI0 => todo!(),
                    Interrupt::SWI1 => todo!(),
                    Interrupt::HWI0
                    | Interrupt::HWI1
                    | Interrupt::HWI2
                    | Interrupt::HWI3
                    | Interrupt::HWI4
                    | Interrupt::HWI5
                    | Interrupt::HWI6
                    | Interrupt::HWI7 => {
                        crate::debug!("Handling hardware interrupt: {:?}", interrupt);
                        while let Some(irq_num) = crate::arch::irq::chip::claim() {
                            // Call the IRQ callback functions for the claimed interrupt
                            call_irq_callback_functions(
                                &self.as_trap_frame(),
                                &HwIrqLine::new(irq_num),
                                PrivilegeLevel::User,
                            );
                        }
                    }
                    Interrupt::PMI => todo!(),
                    Interrupt::Timer => todo!(),
                    Interrupt::IPI => todo!(),
                },
                Trap::MachineError(machine_error) => panic!(
                    "Machine error: {machine_error:?}, badv: {badv:#x?}, badi: {badi:#x?}, era: {era:#x?}"
                ),
                Trap::Unknown if estat.ecode() == ECODE_FPE => {
                    // `loongArch64` 0.2.6 maps the FPE exception code (`0x12`) to `Trap::Unknown`,
                    // so inspect `estat.ecode()` directly. Capture `FCSR` before another task can
                    // overwrite the user register state.
                    let fcsr: u32;
                    // SAFETY: The FPU is enabled because an FPE occurs. Therefore, it is safe to
                    // read from the FCSR.
                    unsafe { asm!("movfcsr2gr {}, $fcsr0", out(reg) fcsr, options(nostack)) };

                    // Clear enabled cause bits in `FCSR` so they do not remain pending in the
                    // saved user state.
                    const FCSR_CAUSE_SHIFT: u32 = 24;
                    const FCSR_ENABLE_MASK: u32 = 0x1f;
                    let enabled_causes = (fcsr & FCSR_ENABLE_MASK) << FCSR_CAUSE_SHIFT;
                    // SAFETY: The FPU is enabled because an FPE occurs. Therefore, it is safe to
                    // write to the FCSR.
                    unsafe {
                        asm!(
                            "movgr2fcsr $fcsr0, {}",
                            in(reg) fcsr & !enabled_causes,
                            options(nostack),
                        )
                    };

                    self.exception = Some(CpuException::FloatingPoint { fcsr });
                    break ReturnReason::UserException;
                }
                Trap::Unknown => {
                    panic!("Unknown trap, badv: {badv:#x?}, badi: {badi:#x?}, era: {era:#x?}")
                }
            }

            drop(irq_guard);

            if hooks.has_kernel_event() {
                break ReturnReason::KernelEvent;
            }
        }
    }

    fn as_trap_frame(&self) -> TrapFrame {
        TrapFrame {
            general: self.user_context.general,
            prmd: self.user_context.prmd,
            era: self.user_context.era,
            euen: self.user_context.euen,
        }
    }
}

impl UserContextApi for UserContext {
    fn instruction_pointer(&self) -> usize {
        self.user_context.era
    }

    fn set_instruction_pointer(&mut self, ip: usize) {
        self.user_context.era = ip;
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
    [tp, set_tp],
    [sp, set_sp],
    [a0, set_a0],
    [a1, set_a1],
    [a2, set_a2],
    [a3, set_a3],
    [a4, set_a4],
    [a5, set_a5],
    [a6, set_a6],
    [a7, set_a7],
    [t0, set_t0],
    [t1, set_t1],
    [t2, set_t2],
    [t3, set_t3],
    [t4, set_t4],
    [t5, set_t5],
    [t6, set_t6],
    [t7, set_t7],
    [t8, set_t8],
    [r21, set_r21],
    [fp, set_fp],
    [s0, set_s0],
    [s1, set_s1],
    [s2, set_s2],
    [s3, set_s3],
    [s4, set_s4],
    [s5, set_s5],
    [s6, set_s6],
    [s7, set_s7],
    [s8, set_s8]
);

/// The floating-point and SIMD context of a user task.
#[derive(Clone, Debug)]
pub struct FpuContext {
    registers: Option<Box<FpSimdRegisters>>,
}

const LOONGARCH_CSR_EUEN: usize = 0x2;
const EUEN_FPE: usize = 1 << 0;
const EUEN_SXE: usize = 1 << 1;
const EUEN_ASXE: usize = 1 << 2;

/// Returns the EUEN bits for the floating-point and SIMD units supported by this CPU.
pub(in crate::arch) fn supported_fpu_units() -> usize {
    const CPUCFG_FEATURES: usize = 2;
    const FP_BIT: usize = 0;
    const LSX_BIT: usize = 6;
    const LASX_BIT: usize = 7;

    let config = loongArch64::cpu::CPUCFG::read(CPUCFG_FEATURES);
    if !config.get_bit(FP_BIT) {
        return 0;
    }

    let mut units = EUEN_FPE;
    if config.get_bit(LSX_BIT) {
        units |= EUEN_SXE;
    }
    if config.get_bit(LASX_BIT) {
        units |= EUEN_ASXE;
    }
    units
}

impl FpuContext {
    /// Creates a new FPU context.
    pub fn new() -> Self {
        Self {
            registers: (supported_fpu_units() != 0).then(Box::default),
        }
    }

    /// Saves CPU's current FPU context to this instance.
    pub fn save(&mut self) {
        let Some(registers) = self.registers.as_deref_mut() else {
            return;
        };

        let _irq_guard = crate::irq::disable_local();

        // SAFETY:
        // 1. Local IRQs are disabled while `_irq_guard` is alive.
        // 2. The FPU/SIMD units returned by `supported_fpu_units()` are supported by the CPU.
        unsafe { save_fpu_context(registers, supported_fpu_units()) };
    }

    /// Loads CPU's FPU context from this instance.
    pub fn load(&self) {
        let Some(registers) = self.registers.as_deref() else {
            return;
        };

        let _irq_guard = crate::irq::disable_local();

        // SAFETY:
        // 1. Local IRQs are disabled while `_irq_guard` is alive.
        // 2. The FPU/SIMD units returned by `supported_fpu_units()` are supported by the CPU.
        unsafe { load_fpu_context(registers, supported_fpu_units()) };
    }

    /// Returns the FPU context as a byte slice.
    pub fn as_bytes(&self) -> &[u8] {
        self.registers.as_deref().map_or(&[], IntoBytes::as_bytes)
    }

    /// Returns the FPU context as a mutable byte slice.
    pub fn as_bytes_mut(&mut self) -> &mut [u8] {
        self.registers
            .as_deref_mut()
            .map_or(&mut [], IntoBytes::as_mut_bytes)
    }
}

impl Default for FpuContext {
    fn default() -> Self {
        Self::new()
    }
}

#[padding_struct]
#[repr(C, align(32))]
#[derive(Clone, Copy, Debug, Default, Pod)]
struct FpSimdRegisters {
    // FP, LSX, or LASX registers, depending on the FPU units supported by the CPU.
    registers: [[u64; 4]; 32],
    fcc: [u8; 8],
    fcsr: u32,
}

core::arch::global_asm!(
    include_str!("fpu.S"),
    LOONGARCH_CSR_EUEN = const LOONGARCH_CSR_EUEN,
    EUEN_SXE = const EUEN_SXE,
    EUEN_ASXE = const EUEN_ASXE,
    REGISTER_STRIDE = const size_of::<[u64; 4]>(),
    FCC_OFFSET = const core::mem::offset_of!(FpSimdRegisters, fcc),
    FCSR_OFFSET = const core::mem::offset_of!(FpSimdRegisters, fcsr),
);

// SAFETY:
// 1. The assembly routines correctly save and load the FPU registers.
// 2. The FPU registers do not affect the kernel's memory safety.
// 3. Since these functions temporarily enable FPU/SIMD instructions in the kernel, they must only
//    be called while local IRQs are disabled and when the CPU supports the units specified in
//    `units`.
unsafe extern "C" {
    /// Saves the current floating-point and SIMD register state.
    ///
    /// # Safety
    ///
    /// Local IRQs must be disabled and the CPU must support the FPU/SIMD units specified in `units`.
    unsafe fn save_fpu_context(registers: &mut FpSimdRegisters, units: usize);

    /// Restores floating-point and SIMD register state.
    ///
    /// # Safety
    ///
    /// Local IRQs must be disabled and the CPU must support the FPU/SIMD units specified in `units`.
    unsafe fn load_fpu_context(registers: &FpSimdRegisters, units: usize);
}
