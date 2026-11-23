// SPDX-License-Identifier: MPL-2.0

//! CPU execution context control.

use alloc::boxed::Box;
use core::{arch::global_asm, fmt::Debug};

use ostd_pod::IntoBytes;
use riscv::{
    interrupt::supervisor::{Exception, Interrupt},
    register::scause::Trap,
};

use crate::{
    arch::{
        cpu::extension::{IsaExtensions, has_extensions},
        trap::{RawUserContext, SSTATUS_FS_MASK, TrapFrame, handle_irq},
    },
    cpu::PrivilegeLevel,
    user::{ReturnReason, UserContextApi, UserContextApiInternal, UserModeHooks},
};

/// Userspace CPU context, including general-purpose registers and exception information.
#[repr(C)]
#[derive(Clone, Debug, Default)]
pub struct UserContext {
    user_context: RawUserContext,
    exception: Option<CpuException>,
}

/// General registers.
#[expect(missing_docs)]
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
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

/// RISC-V CPU exceptions.
///
/// Every enum variant corresponds to one exception defined by the RISC-V
/// architecture. Variants that naturally carry an error code (in stval
/// register) expose it through their associated data fields.
#[derive(Clone, Copy, Debug)]
pub enum CpuException {
    /// Instruction address misalignment exception.
    InstructionMisaligned,
    /// Instruction access fault exception.
    InstructionFault,
    /// Illegal instruction exception.
    IllegalInstruction(FaultInstruction),
    /// Breakpoint exception.
    Breakpoint,
    /// Load address misalignment exception.
    LoadMisaligned(FaultAddress),
    /// Load access fault exception.
    LoadFault(FaultAddress),
    /// Store address misalignment exception.
    StoreMisaligned(FaultAddress),
    /// Store access fault exception.
    StoreFault(FaultAddress),
    /// Environment call from user mode.
    UserEnvCall,
    /// Environment call from supervisor mode.
    SupervisorEnvCall,
    /// Instruction page fault exception.
    InstructionPageFault(FaultAddress),
    /// Load page fault exception.
    LoadPageFault(FaultAddress),
    /// Store page fault exception.
    StorePageFault(FaultAddress),
    /// Unknown.
    Unknown,
}

impl CpuException {
    pub(in crate::arch) fn new(raw_exception: Exception, stval: usize) -> Self {
        use Exception::*;

        match raw_exception {
            InstructionMisaligned => Self::InstructionMisaligned,
            InstructionFault => Self::InstructionFault,
            IllegalInstruction => Self::IllegalInstruction({
                if stval & 0x3 == 0x3 {
                    FaultInstruction::Normal(stval as u32)
                } else {
                    FaultInstruction::Compressed(stval as u16)
                }
            }),
            Breakpoint => Self::Breakpoint,
            LoadMisaligned => Self::LoadMisaligned(stval),
            LoadFault => Self::LoadFault(stval),
            StoreMisaligned => Self::StoreMisaligned(stval),
            StoreFault => Self::StoreFault(stval),
            UserEnvCall => Self::UserEnvCall,
            SupervisorEnvCall => Self::SupervisorEnvCall,
            InstructionPageFault => Self::InstructionPageFault(stval),
            LoadPageFault => Self::LoadPageFault(stval),
            StorePageFault => Self::StorePageFault(stval),
        }
    }
}

/// Data address of data access exceptions.
pub type FaultAddress = usize;

/// Illegal instruction that caused exception.
#[derive(Clone, Copy, Debug)]
pub enum FaultInstruction {
    /// Normal 4-byte instruction.
    Normal(u32),
    /// Compressed 2-byte instruction. Used only when compressed (C) extension
    /// is enabled.
    Compressed(u16),
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

            let scause = riscv::register::scause::read();
            let Ok(cause) = Trap::<Interrupt, Exception>::try_from(scause.cause()) else {
                match scause.cause() {
                    Trap::Interrupt(i) => {
                        panic!(
                            "Cannot handle unknown interrupt: {:#x?}; trapframe: {:#x?}",
                            i,
                            self.as_trap_frame()
                        );
                    }
                    Trap::Exception(e) => {
                        crate::info!("Unknown CPU exception in user mode: {:#x?}", e);
                        self.exception = Some(CpuException::Unknown);
                        break ReturnReason::UserException;
                    }
                }
            };
            match cause {
                Trap::Interrupt(interrupt) => {
                    handle_irq(&self.as_trap_frame(), interrupt, PrivilegeLevel::User);
                    drop(irq_guard);
                }
                Trap::Exception(Exception::UserEnvCall) => {
                    drop(irq_guard);
                    self.user_context.sepc += 4;
                    break ReturnReason::UserSyscall;
                }
                Trap::Exception(raw_exception) => {
                    let stval = riscv::register::stval::read();
                    drop(irq_guard);
                    let exception = CpuException::new(raw_exception, stval);
                    self.exception = Some(exception);
                    break ReturnReason::UserException;
                }
            }

            if hooks.has_kernel_event() {
                break ReturnReason::KernelEvent;
            }
        }
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

/// The FPU context of a user task.
#[derive(Clone, Debug)]
pub struct FpuContext {
    inner: FpuContextInner,
}

#[derive(Clone, Debug)]
enum FpuContextInner {
    /// FPU context for F extension (32-bit floating point).
    F(Box<FFpuContext>),
    /// FPU context for D extension (64-bit floating point).
    D(Box<DFpuContext>),
    /// FPU context for Q extension (128-bit floating point).
    Q(Box<QFpuContext>),
    /// No FPU context (no FPU extensions enabled).
    None,
}

impl FpuContext {
    /// The maximum size of an FPU context in bytes.
    ///
    /// [`Self::as_bytes`] and [`Self::as_bytes_mut`] are guaranteed to return
    /// a slice of a length that is equal to or less than this value.
    pub const MAX_SIZE: usize = size_of::<QFpuContext>();

    /// Creates a new FPU context.
    pub fn new() -> Self {
        let inner = if has_extensions(IsaExtensions::Q) {
            FpuContextInner::Q(Box::default())
        } else if has_extensions(IsaExtensions::D) {
            FpuContextInner::D(Box::default())
        } else if has_extensions(IsaExtensions::F) {
            FpuContextInner::F(Box::default())
        } else {
            FpuContextInner::None
        };
        Self { inner }
    }

    /// Saves CPU's current FPU context to this instance.
    pub fn save(&mut self) {
        let _irq_guard = crate::irq::disable_local();

        // SAFETY:
        // 1. Local IRQs are disabled while `_irq_guard` is alive.
        // 2. The FPU extension is available, as was checked when constructing `self`.
        unsafe {
            match &mut self.inner {
                FpuContextInner::F(ctx) => save_fpu_context_f(ctx.as_mut()),
                FpuContextInner::D(ctx) => save_fpu_context_d(ctx.as_mut()),
                FpuContextInner::Q(ctx) => save_fpu_context_q(ctx.as_mut()),
                FpuContextInner::None => {}
            }
        }
    }

    /// Loads CPU's FPU context from this instance.
    pub fn load(&self) {
        let _irq_guard = crate::irq::disable_local();

        // SAFETY:
        // 1. Local IRQs are disabled while `_irq_guard` is alive.
        // 2. The FPU extension is available, as was checked when constructing `self`.
        unsafe {
            match &self.inner {
                FpuContextInner::F(ctx) => load_fpu_context_f(ctx.as_ref()),
                FpuContextInner::D(ctx) => load_fpu_context_d(ctx.as_ref()),
                FpuContextInner::Q(ctx) => load_fpu_context_q(ctx.as_ref()),
                FpuContextInner::None => {}
            }
        }
    }

    /// Returns the FPU context as a byte slice.
    pub fn as_bytes(&self) -> &[u8] {
        match &self.inner {
            FpuContextInner::F(ctx) => ctx.as_bytes(),
            FpuContextInner::D(ctx) => ctx.as_bytes(),
            FpuContextInner::Q(ctx) => ctx.as_bytes(),
            FpuContextInner::None => &[],
        }
    }

    /// Returns the FPU context as a mutable byte slice.
    pub fn as_bytes_mut(&mut self) -> &mut [u8] {
        match &mut self.inner {
            FpuContextInner::F(ctx) => ctx.as_mut_bytes(),
            FpuContextInner::D(ctx) => ctx.as_mut_bytes(),
            FpuContextInner::Q(ctx) => ctx.as_mut_bytes(),
            FpuContextInner::None => &mut [],
        }
    }
}

impl Default for FpuContext {
    fn default() -> Self {
        Self::new()
    }
}

/// FPU context for F extension (32-bit floating point).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
struct FFpuContext {
    f: [u32; 32],
    fcsr: u32,
}

/// FPU context for D extension (64-bit floating point).
#[padding_struct]
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
struct DFpuContext {
    f: [u64; 32],
    fcsr: u32,
}

/// FPU context for Q extension (128-bit floating point).
#[padding_struct]
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
struct QFpuContext {
    f: [u64; 64],
    fcsr: u32,
}

// FIXME: Currently Rust generates array impls for every size up to 32 manually
// and there is ongoing work on refactoring with const generics. We can just
// derive the `Default` implementation once that is done.
//
// See <https://github.com/rust-lang/rust/issues/61415>.
impl Default for QFpuContext {
    fn default() -> Self {
        Self {
            f: [0; 64],
            __pad1: [0; _],
            fcsr: 0,
            __pad2: [0; _],
        }
    }
}

global_asm!(include_str!("fpu.S"), SSTATUS_FS_MASK = const SSTATUS_FS_MASK);

// SAFETY:
// 1. The assembly routines correctly save and load the FPU registers.
// 2. The FPU registers do not affect the kernel's memory safety.
// 3. Since these functions temporarily enable FPU instructions in the kernel, they must only be
//    called while local IRQs are disabled and when the CPU supports the corresponding FPU
//    extension.
unsafe extern "C" {
    /// Saves the current single-precision floating-point register state.
    ///
    /// # Safety
    ///
    /// Local IRQs must be disabled and the CPU must support F extension.
    unsafe fn save_fpu_context_f(ctx: &mut FFpuContext);

    /// Restores the single-precision floating-point register state.
    ///
    /// # Safety
    ///
    /// Local IRQs must be disabled and the CPU must support F extension.
    unsafe fn load_fpu_context_f(ctx: &FFpuContext);

    /// Saves the current double-precision floating-point register state.
    ///
    /// # Safety
    ///
    /// Local IRQs must be disabled and the CPU must support D extension.
    unsafe fn save_fpu_context_d(ctx: &mut DFpuContext);

    /// Restores the double-precision floating-point register state.
    ///
    /// # Safety
    ///
    /// Local IRQs must be disabled and the CPU must support D extension.
    unsafe fn load_fpu_context_d(ctx: &DFpuContext);

    /// Saves the current quad-precision floating-point register state.
    ///
    /// # Safety
    ///
    /// Local IRQs must be disabled and the CPU must support Q extension.
    unsafe fn save_fpu_context_q(ctx: &mut QFpuContext);

    /// Restores the quad-precision floating-point register state.
    ///
    /// # Safety
    ///
    /// Local IRQs must be disabled and the CPU must support Q extension.
    unsafe fn load_fpu_context_q(ctx: &QFpuContext);
}
