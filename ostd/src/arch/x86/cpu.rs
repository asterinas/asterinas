// SPDX-License-Identifier: MPL-2.0

//! CPU.

use alloc::vec::Vec;
use core::{
    arch::x86_64::{_fxrstor, _fxsave},
    fmt::Debug,
};

use bitflags::bitflags;
use bitvec::{
    prelude::{BitVec, Lsb0},
    slice::IterOnes,
};
use log::debug;
pub use trapframe::GeneralRegs as RawGeneralRegs;
use trapframe::UserContext as RawUserContext;
use x86_64::registers::{
    rflags::RFlags,
    segmentation::{Segment64, FS},
};

use crate::{
    exception::user_mode_exception_handler,
    user::{ReturnReason, UserContextApi, UserContextApiInternal},
};

/// Returns the number of CPUs.
pub fn num_cpus() -> u32 {
    // FIXME: we only start one cpu now.
    1
}

/// Returns the ID of this CPU.
pub fn this_cpu() -> u32 {
    // FIXME: we only start one cpu now.
    0
}

/// A set of CPUs.
#[derive(Default)]
pub struct CpuSet {
    bitset: BitVec,
}

impl CpuSet {
    /// Creates a new `CpuSet` with all CPUs included.
    pub fn new_full() -> Self {
        let num_cpus = num_cpus();
        let mut bitset = BitVec::with_capacity(num_cpus as usize);
        bitset.resize(num_cpus as usize, true);
        Self { bitset }
    }

    /// Creates a new `CpuSet` with no CPUs included.
    pub fn new_empty() -> Self {
        let num_cpus = num_cpus();
        let mut bitset = BitVec::with_capacity(num_cpus as usize);
        bitset.resize(num_cpus as usize, false);
        Self { bitset }
    }

    /// Adds a CPU with identifier `cpu_id` to the `CpuSet`.
    pub fn add(&mut self, cpu_id: u32) {
        self.bitset.set(cpu_id as usize, true);
    }

    /// Adds multiple CPUs from `cpu_ids` to the `CpuSet`.
    pub fn add_from_vec(&mut self, cpu_ids: Vec<u32>) {
        for cpu_id in cpu_ids {
            self.add(cpu_id)
        }
    }

    /// Adds all available CPUs to the `CpuSet`.
    pub fn add_all(&mut self) {
        self.bitset.fill(true);
    }

    /// Removes a CPU with identifier `cpu_id` from the `CpuSet`.
    pub fn remove(&mut self, cpu_id: u32) {
        self.bitset.set(cpu_id as usize, false);
    }

    /// Removes multiple CPUs from `cpu_ids` from the `CpuSet`.
    pub fn remove_from_vec(&mut self, cpu_ids: Vec<u32>) {
        for cpu_id in cpu_ids {
            self.remove(cpu_id);
        }
    }

    /// Clears the `CpuSet`, removing all CPUs.
    pub fn clear(&mut self) {
        self.bitset.fill(false);
    }

    /// Checks if the `CpuSet` contains a specific CPU.
    pub fn contains(&self, cpu_id: u32) -> bool {
        self.bitset.get(cpu_id as usize).as_deref() == Some(&true)
    }

    /// Returns an iterator over the set CPUs.
    pub fn iter(&self) -> IterOnes<'_, usize, Lsb0> {
        self.bitset.iter_ones()
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

#[cfg(feature = "intel_tdx")]
impl crate::arch::tdx_guest::TdxTrapFrame for RawGeneralRegs {
    fn rax(&self) -> usize {
        self.rax
    }
    fn set_rax(&mut self, rax: usize) {
        self.rax = rax;
    }
    fn rbx(&self) -> usize {
        self.rbx
    }
    fn set_rbx(&mut self, rbx: usize) {
        self.rbx = rbx;
    }
    fn rcx(&self) -> usize {
        self.rcx
    }
    fn set_rcx(&mut self, rcx: usize) {
        self.rcx = rcx;
    }
    fn rdx(&self) -> usize {
        self.rdx
    }
    fn set_rdx(&mut self, rdx: usize) {
        self.rdx = rdx;
    }
    fn rsi(&self) -> usize {
        self.rsi
    }
    fn set_rsi(&mut self, rsi: usize) {
        self.rsi = rsi;
    }
    fn rdi(&self) -> usize {
        self.rdi
    }
    fn set_rdi(&mut self, rdi: usize) {
        self.rdi = rdi;
    }
    fn rip(&self) -> usize {
        self.rip
    }
    fn set_rip(&mut self, rip: usize) {
        self.rip = rip;
    }
    fn r8(&self) -> usize {
        self.r8
    }
    fn set_r8(&mut self, r8: usize) {
        self.r8 = r8;
    }
    fn r9(&self) -> usize {
        self.r9
    }
    fn set_r9(&mut self, r9: usize) {
        self.r9 = r9;
    }
    fn r10(&self) -> usize {
        self.r10
    }
    fn set_r10(&mut self, r10: usize) {
        self.r10 = r10;
    }
    fn r11(&self) -> usize {
        self.r11
    }
    fn set_r11(&mut self, r11: usize) {
        self.r11 = r11;
    }
    fn r12(&self) -> usize {
        self.r12
    }
    fn set_r12(&mut self, r12: usize) {
        self.r12 = r12;
    }
    fn r13(&self) -> usize {
        self.r13
    }
    fn set_r13(&mut self, r13: usize) {
        self.r13 = r13;
    }
    fn r14(&self) -> usize {
        self.r14
    }
    fn set_r14(&mut self, r14: usize) {
        self.r14 = r14;
    }
    fn r15(&self) -> usize {
        self.r15
    }
    fn set_r15(&mut self, r15: usize) {
        self.r15 = r15;
    }
    fn rbp(&self) -> usize {
        self.rbp
    }
    fn set_rbp(&mut self, rbp: usize) {
        self.rbp = rbp;
    }
}

/// User Preemption.
pub struct UserPreemption {
    count: u32,
}

impl UserPreemption {
    const PREEMPTION_INTERVAL: u32 = 100;

    /// Creates a new instance of `UserPreemption`.
    #[allow(clippy::new_without_default)]
    pub const fn new() -> Self {
        UserPreemption { count: 0 }
    }

    /// Checks if preemption might occur and takes necessary actions.
    pub fn might_preempt(&mut self) {
        self.count = (self.count + 1) % Self::PREEMPTION_INTERVAL;

        if self.count == 0 {
            crate::task::schedule();
        }
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

    /// Returns a reference to the floating point registers
    pub fn fp_regs(&self) -> &FpRegs {
        &self.fp_regs
    }

    /// Returns a mutable reference to the floating point registers
    pub fn fp_regs_mut(&mut self) -> &mut FpRegs {
        &mut self.fp_regs
    }
}

impl UserContextApiInternal for UserContext {
    fn execute<F>(&mut self, mut has_kernel_event: F) -> ReturnReason
    where
        F: FnMut() -> bool,
    {
        // Make sure that when when calling `execute`, the caller does not hold
        // a guard disabling the local IRQ.
        // TODO: we may introduce a compile time check here to forbid OSTD users
        // doing so. This is currently a debug assertion to save performance.
        debug_assert!(crate::arch::irq::is_local_enabled());

        // Set interrupt flag so that in user mode it can receive external interrupts.
        // Set ID flag which means cpu support CPUID instruction.
        self.user_context.general.rflags |= (RFlags::INTERRUPT_FLAG | RFlags::ID).bits() as usize;

        let mut user_preemption = UserPreemption::new();
        // Go back to user when it is syscall or cpu exception type is an immediately
        // recoverable Fault or Trap. Otherwise return to handle the exception.
        let return_reason = loop {
            self.user_context.run();

            if let Some(return_reason) = user_mode_exception_handler(self) {
                break return_reason;
            }

            if has_kernel_event() {
                break ReturnReason::KernelEvent;
            }

            user_preemption.might_preempt();
        };

        if return_reason == ReturnReason::UserException {
            self.cpu_exception_info = CpuExceptionInfo {
                page_fault_addr: unsafe { x86::controlregs::cr2() },
                id: self.user_context.trap_num,
                error_code: self.user_context.error_code,
            };
        }

        return_reason
    }

    fn as_trap_frame(&self) -> trapframe::TrapFrame {
        trapframe::TrapFrame {
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

/// CPU exceptions.
///
/// This summarizes all the "abormal" control flow events that can occur in the
/// CPU, which covers traps, faults and interrupts in the traditional sense.
#[derive(PartialEq, Eq, Debug)]
pub enum CpuExceptionType {
    /// CPU faults.
    ///
    /// Faults can be corrected, and the program may continue as if nothing happened.
    Fault,
    /// CPU traps.
    ///
    /// Traps are reported immediately after the execution of the trapping instruction.
    Trap,
    /// Faults or traps.
    ///
    /// This indicates that this event can be either a fault or a trap.
    FaultOrTrap,
    /// User-interpreted interrupts caused by either hardwares or softwares.
    Interrupt,
    /// Some severe unrecoverable error.
    Abort,
    /// Reserved exception type.
    ///
    /// Such a type won't be used or allowed to become user-interpreted interrupts. It's
    /// generally unacceptable if happening.
    Reserved,
}

macro_rules! define_cpu_exception {
    ( $([ $name: ident = $exception_num:expr, $exception_type:tt]),* ) => {
        /// A CPU exception.
        #[derive(Debug, Eq, PartialEq)]
        pub enum CpuException {
            $(
                #[doc = concat!("The ", stringify!($name), " exception.")]
                $name,
            )*
            /// This exception number is not explicitly defined in the ISA.
            /// It may be interpreted as an user-defined interrupt.
            NotExplicitInISA(u16),
        }

        impl CpuException {
            /// Get the type of the exception.
            pub fn get_type(&self) -> CpuExceptionType {
                match self {
                    $(
                        Self::$name => CpuExceptionType::$exception_type,
                    )*
                    Self::NotExplicitInISA(n) => {
                        if *n >= 32 && *n < 256 {
                            CpuExceptionType::Interrupt
                        } else {
                            CpuExceptionType::Reserved
                        }
                    }
                }
            }

            /// Get the exception from the exception number.
            pub fn from_num(num: u16) -> Self {
                match num {
                    $(
                        $exception_num => Self::$name,
                    )*
                    n => CpuException::NotExplicitInISA(n),
                }
            }
        }
    }
}

/// The exception number when `syscall` is executed by the user.
pub static SYSCALL_EXCEPTION_NUM: u16 = 0x100;

define_cpu_exception!(
    // Hardware exceptions
    [DivideByZero = 0, Fault],
    [Debug = 1, FaultOrTrap],
    [NonMaskableInterrupt = 2, Abort],
    [Breakpoint = 3, Trap],
    [Overflow = 4, Trap],
    [BoundRangeExceeded = 5, Fault],
    [InvalidOpcode = 6, Fault],
    [DeviceNotAvailable = 7, Fault],
    [DoubleFault = 8, Abort],
    [CoprocessorSegmentOverrun = 9, Fault], // Not supported on AMD64
    [InvalidTss = 10, Fault],
    [SegmentNotPresent = 11, Fault],
    [StackSegmentFault = 12, Fault],
    [GeneralProtectionFault = 13, Fault],
    [PageFault = 14, Fault],
    [X87FloatingPointException = 16, Fault],
    [AlignmentCheck = 17, Fault],
    [MachineCheck = 18, Abort],
    [SimdFloatingPointException = 19, Fault],
    [VirtualizationException = 20, Fault],
    [ControlProtectionException = 21, Fault],
    [HypervisorInjectionException = 28, Fault],
    [VmmCommunicationException = 29, Fault],
    [SecurityException = 30, Fault]
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

/// Sets the base address for the CPU local storage by writing to the FS base model-specific register.
/// This operation is marked as `unsafe` because it directly interfaces with low-level CPU registers.
///
/// # Safety
///
///  - This function is safe to call provided that the FS register is dedicated entirely for CPU local storage
///    and is not concurrently accessed for other purposes.
///  - The caller must ensure that `addr` is a valid address and properly aligned, as required by the CPU.
///  - This function should only be called in contexts where the CPU is in a state to accept such changes,
///    such as during processor initialization.
pub(crate) unsafe fn set_cpu_local_base(addr: u64) {
    FS::write_base(x86_64::addr::VirtAddr::new(addr));
}

/// Gets the base address for the CPU local storage by reading the FS base model-specific register.
pub(crate) fn get_cpu_local_base() -> u64 {
    FS::read_base().as_u64()
}
