// SPDX-License-Identifier: MPL-2.0

//! CPU execution context control.

use core::{arch::global_asm, fmt::Debug, sync::atomic::Ordering};

use riscv::register::scause::Exception;

use crate::{
    arch::{
        cpu::extension::{has_extensions, IsaExtensions},
        irq::IRQ_CHIP,
        trap::{RawUserContext, TrapFrame},
        TIMER_IRQ_NUM,
    },
    cpu::{CpuId, PrivilegeLevel},
    irq::call_irq_callback_functions,
    task::scheduler,
    user::{ReturnReason, UserContextApi, UserContextApiInternal},
};

/// Userspace CPU context, including general-purpose registers and exception information.
#[derive(Clone, Default, Debug)]
#[repr(C)]
pub struct UserContext {
    user_context: RawUserContext,
    exception: Option<CpuException>,
}

/// General-purpose registers.
#[derive(Debug, Default, Clone, Copy)]
#[repr(C)]
#[expect(missing_docs)]
pub struct GeneralRegs {
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

/// RISC-V CPU exceptions
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
    IllegalInstruction(Instruction),
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
    /// Environment call from machine mode.
    MachineEnvCall,
    /// Instruction page fault exception.
    InstructionPageFault(FaultAddress),
    /// Load page fault exception.
    LoadPageFault(FaultAddress),
    /// Store page fault exception.
    StorePageFault(FaultAddress),
    /// Unknown.
    Unknown,
}

impl From<Exception> for CpuException {
    fn from(value: Exception) -> Self {
        use Exception::*;

        let stval = riscv::register::stval::read();
        match value {
            InstructionMisaligned => Self::InstructionMisaligned,
            InstructionFault => Self::InstructionFault,
            IllegalInstruction => Self::IllegalInstruction({
                if stval & 0x3 == 0x3 {
                    Instruction::Normal(stval as u32)
                } else {
                    Instruction::Compressed(stval as u16)
                }
            }),
            Breakpoint => Self::Breakpoint,
            LoadMisaligned => Self::LoadMisaligned(FaultAddress(stval)),
            LoadFault => Self::LoadFault(FaultAddress(stval)),
            StoreMisaligned => Self::StoreMisaligned(FaultAddress(stval)),
            StoreFault => Self::StoreFault(FaultAddress(stval)),
            UserEnvCall => Self::UserEnvCall,
            SupervisorEnvCall => Self::SupervisorEnvCall,
            InstructionPageFault => Self::InstructionPageFault(FaultAddress(stval)),
            LoadPageFault => Self::LoadPageFault(FaultAddress(stval)),
            StorePageFault => Self::StorePageFault(FaultAddress(stval)),
            Unknown => Self::Unknown,
        }
    }
}

#[derive(Clone, Copy, Debug)]
/// Data address of data access exceptions.
pub struct FaultAddress(pub usize);

#[derive(Clone, Copy, Debug)]
/// Illegal instruction that caused exception.
pub enum Instruction {
    /// Normal 4-byte instruction.
    Normal(u32),
    /// Compressed 2-byte instruction. Used only when compressed (C) extension
    /// is enabled.
    Compressed(u16),
}

impl UserContext {
    /// Returns a reference to the general registers.
    pub fn general_regs(&self) -> &GeneralRegs {
        &self.user_context.general_regs
    }

    /// Returns a mutable reference to the general registers
    pub fn general_regs_mut(&mut self) -> &mut GeneralRegs {
        &mut self.user_context.general_regs
    }

    /// Returns the trap information.
    pub fn take_exception(&mut self) -> Option<CpuException> {
        self.exception.take()
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
        use riscv::register::scause::{Interrupt::*, Trap::*};

        // Enable userspace traps.
        self.user_context.sstatus |= 1 << 5;
        // Set FPU state to clean.
        self.user_context.sstatus |= 2 << 13;
        loop {
            scheduler::might_preempt();
            self.user_context.run();

            let scause = riscv::register::scause::read();
            match scause.cause() {
                Interrupt(SupervisorTimer) => {
                    call_irq_callback_functions(
                        &self.as_trap_frame(),
                        TIMER_IRQ_NUM.load(Ordering::Relaxed) as usize,
                        PrivilegeLevel::User,
                    );
                }
                Interrupt(SupervisorExternal) => {
                    let current_cpu = CpuId::current_racy().as_usize() as u32;
                    loop {
                        let irq_chip = IRQ_CHIP.get().unwrap().lock();
                        match irq_chip.claim_interrupt(current_cpu) {
                            Some(irq_num) => {
                                drop(irq_chip);
                                call_irq_callback_functions(
                                    &self.as_trap_frame(),
                                    irq_num as usize,
                                    PrivilegeLevel::User,
                                );
                            }
                            None => break,
                        }
                    }
                }
                Interrupt(SupervisorSoft) => todo!(),
                Interrupt(Unknown) => {
                    panic!(
                        "Cannot handle unknown supervisor interrupt, scause: {:#x}, trapframe: {:?}.",
                        scause.bits(),
                        self.as_trap_frame()
                    );
                }
                Exception(e) => {
                    use CpuException::*;

                    let exception = e.into();
                    crate::arch::irq::enable_local();
                    match exception {
                        UserEnvCall => {
                            self.user_context.sepc += 4;
                            return ReturnReason::UserSyscall;
                        }
                        Unknown => {
                            panic!(
                                "Cannot handle unknown exception, scause: {:#x}, trapframe: {:?}.",
                                scause.bits(),
                                self.as_trap_frame()
                            );
                        }
                        _ => {
                            self.exception = Some(exception);
                            return ReturnReason::UserException;
                        }
                    }
                }
            }
            crate::arch::irq::enable_local();

            if has_kernel_event() {
                break ReturnReason::KernelEvent;
            }
        }
    }

    fn as_trap_frame(&self) -> TrapFrame {
        TrapFrame {
            general_regs: self.user_context.general_regs,
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
                    self.user_context.general_regs.$field
                }

                #[doc = concat!("Sets the value of ", stringify!($field))]
                #[inline(always)]
                pub fn $setter_name(&mut self, $field: usize) {
                    self.user_context.general_regs.$field = $field;
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

/// The FPU context of user task.
#[derive(Clone, Debug)]
pub enum FpuContext {
    /// FPU context for F extension (32-bit floating point).
    F(FFpuContext),
    /// FPU context for D extension (64-bit floating point).
    D(DFpuContext),
    /// FPU context for Q extension (128-bit floating point).
    Q(QFpuContext),
}

impl FpuContext {
    /// Creates a new FPU context.
    pub fn new() -> Self {
        if has_extensions(IsaExtensions::Q) {
            Self::Q(QFpuContext::default())
        } else if has_extensions(IsaExtensions::D) {
            Self::D(DFpuContext::default())
        } else if has_extensions(IsaExtensions::F) {
            Self::F(FFpuContext::default())
        } else {
            panic!("No FPU extensions enabled");
        }
    }

    /// Saves CPU's current FPU context to this instance.
    pub fn save(&mut self) {
        match self {
            Self::F(ctx) => ctx.save(),
            Self::D(ctx) => ctx.save(),
            Self::Q(ctx) => ctx.save(),
        }
    }

    /// Loads CPU's FPU context from this instance.
    pub fn load(&mut self) {
        match self {
            Self::F(ctx) => ctx.load(),
            Self::D(ctx) => ctx.load(),
            Self::Q(ctx) => ctx.load(),
        }
    }

    /// Returns the FPU context as a byte slice.
    pub fn as_bytes(&self) -> &[u8] {
        match self {
            Self::F(ctx) => ctx.as_bytes(),
            Self::D(ctx) => ctx.as_bytes(),
            Self::Q(ctx) => ctx.as_bytes(),
        }
    }

    /// Returns the FPU context as a mutable byte slice.
    pub fn as_bytes_mut(&mut self) -> &mut [u8] {
        match self {
            Self::F(ctx) => ctx.as_bytes_mut(),
            Self::D(ctx) => ctx.as_bytes_mut(),
            Self::Q(ctx) => ctx.as_bytes_mut(),
        }
    }

    // Linux uses a union of three kinds of FPU context here. We follow the same
    // layout to be compatible with libraries that supports Linux.
    const F_FPU_STATE_SIZE: usize = size_of::<u32>() * 32 + size_of::<u32>();
    const D_FPU_STATE_SIZE: usize = size_of::<u64>() * 32 + size_of::<u32>();
    const Q_FPU_STATE_SIZE: usize = size_of::<u64>() * 64 + size_of::<u32>();
    const F_FPU_CONTEXT_RESERVED_LENGTH: usize =
        (Self::FPU_CONTEXT_SIZE - Self::F_FPU_STATE_SIZE) / size_of::<u32>();
    const D_FPU_CONTEXT_RESERVED_LENGTH: usize =
        (Self::FPU_CONTEXT_SIZE - Self::D_FPU_STATE_SIZE) / size_of::<u32>();
    const Q_FPU_CONTEXT_RESERVED_LENGTH: usize = 3;
    const FPU_CONTEXT_SIZE: usize =
        Self::Q_FPU_STATE_SIZE + size_of::<u32>() * Self::Q_FPU_CONTEXT_RESERVED_LENGTH;
}

impl Default for FpuContext {
    fn default() -> Self {
        Self::new()
    }
}

/// FPU context for F extension (32-bit floating point).
#[repr(C)]
#[derive(Clone, Debug)]
pub struct FFpuContext {
    f: [u32; 32],
    fcsr: u32,
    reserved: [u32; FpuContext::F_FPU_CONTEXT_RESERVED_LENGTH],
}

/// FPU context for D extension (64-bit floating point).
#[repr(C)]
#[derive(Clone, Debug)]
pub struct DFpuContext {
    f: [u64; 32],
    fcsr: u32,
    reserved: [u32; FpuContext::D_FPU_CONTEXT_RESERVED_LENGTH],
}

/// FPU context for Q extension (128-bit floating point).
#[repr(C)]
#[repr(align(16))]
#[derive(Clone, Debug)]
pub struct QFpuContext {
    f: [u64; 64],
    fcsr: u32,
    reserved: [u32; FpuContext::Q_FPU_CONTEXT_RESERVED_LENGTH],
}

macro_rules! impl_fpu_context {
    ($name:ident, $f_length:literal, $reserved_length:expr, $suffix:literal) => {
        impl $name {
            fn save(&mut self) {
                unsafe {
                    paste::paste! {
                        [<save_fpu_context_$suffix>](self as *mut _);
                    }
                }
            }

            fn load(&self) {
                unsafe {
                    paste::paste! {
                        [<load_fpu_context_$suffix>](self as *const _);
                    }
                }
            }

            fn as_bytes(&self) -> &[u8] {
                unsafe {
                    core::slice::from_raw_parts(
                        self as *const Self as *const u8,
                        core::mem::size_of::<Self>(),
                    )
                }
            }

            fn as_bytes_mut(&mut self) -> &mut [u8] {
                unsafe {
                    core::slice::from_raw_parts_mut(
                        self as *mut Self as *mut u8,
                        core::mem::size_of::<Self>(),
                    )
                }
            }

            // Currently Rust generates array impls for every size up to 32
            // manually and there is ongoing work on refactoring with const
            // generics. We can remove the `new` method and just derive the
            // `Default` implementation once that is done.
            //
            // See https://github.com/rust-lang/rust/issues/61415.
            fn new() -> Self {
                Self {
                    f: [0; $f_length],
                    fcsr: 0,
                    reserved: [0; $reserved_length],
                }
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }
    };
}

impl_fpu_context!(
    FFpuContext,
    32,
    FpuContext::F_FPU_CONTEXT_RESERVED_LENGTH,
    "f"
);
impl_fpu_context!(
    DFpuContext,
    32,
    FpuContext::D_FPU_CONTEXT_RESERVED_LENGTH,
    "d"
);
impl_fpu_context!(
    QFpuContext,
    64,
    FpuContext::Q_FPU_CONTEXT_RESERVED_LENGTH,
    "q"
);

global_asm!(include_str!("fpu.S"));

extern "C" {
    fn save_fpu_context_f(ctx: *mut FFpuContext);
    fn load_fpu_context_f(ctx: *const FFpuContext);
    fn save_fpu_context_d(ctx: *mut DFpuContext);
    fn load_fpu_context_d(ctx: *const DFpuContext);
    fn save_fpu_context_q(ctx: *mut QFpuContext);
    fn load_fpu_context_q(ctx: *const QFpuContext);
}
