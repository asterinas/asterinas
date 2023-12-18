//! CPU.

use core::arch::x86_64::{_fxrstor, _fxsave};
use core::fmt::Debug;

use alloc::vec::Vec;
use bitvec::{prelude::Lsb0, slice::IterOnes};
use trapframe::{GeneralRegs, UserContext as RawUserContext};

#[cfg(feature = "intel_tdx")]
use crate::arch::tdx_guest::{handle_virtual_exception, TdxTrapFrame};
use bitvec::prelude::BitVec;
use log::debug;
#[cfg(feature = "intel_tdx")]
use tdx_guest::tdcall;
use x86_64::registers::rflags::RFlags;

use crate::trap::call_irq_callback_functions;
use crate::user::{UserContextApi, UserContextApiInternal, UserEvent};

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

#[derive(Default)]
pub struct CpuSet {
    bitset: BitVec,
}

impl CpuSet {
    pub fn new_full() -> Self {
        let num_cpus = num_cpus();
        let mut bitset = BitVec::with_capacity(num_cpus as usize);
        bitset.resize(num_cpus as usize, true);
        Self { bitset }
    }

    pub fn new_empty() -> Self {
        let num_cpus = num_cpus();
        let mut bitset = BitVec::with_capacity(num_cpus as usize);
        bitset.resize(num_cpus as usize, false);
        Self { bitset }
    }

    pub fn add(&mut self, cpu_id: u32) {
        self.bitset.set(cpu_id as usize, true);
    }

    pub fn add_from_vec(&mut self, cpu_ids: Vec<u32>) {
        for cpu_id in cpu_ids {
            self.add(cpu_id)
        }
    }

    pub fn add_all(&mut self) {
        self.bitset.fill(true);
    }

    pub fn remove(&mut self, cpu_id: u32) {
        self.bitset.set(cpu_id as usize, false);
    }

    pub fn remove_from_vec(&mut self, cpu_ids: Vec<u32>) {
        for cpu_id in cpu_ids {
            self.remove(cpu_id);
        }
    }

    pub fn clear(&mut self) {
        self.bitset.fill(false);
    }

    pub fn contains(&self, cpu_id: u32) -> bool {
        self.bitset.get(cpu_id as usize).as_deref() == Some(&true)
    }

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

#[derive(Clone, Default, Copy, Debug)]
#[repr(C)]
pub struct CpuExceptionInfo {
    pub id: usize,
    pub error_code: usize,
    pub page_fault_addr: usize,
}

#[cfg(feature = "intel_tdx")]
impl TdxTrapFrame for GeneralRegs {
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
}

impl UserContext {
    pub fn general_regs(&self) -> &GeneralRegs {
        &self.user_context.general
    }

    pub fn general_regs_mut(&mut self) -> &mut GeneralRegs {
        &mut self.user_context.general
    }

    pub fn trap_information(&self) -> &CpuExceptionInfo {
        &self.cpu_exception_info
    }

    pub fn fp_regs(&self) -> &FpRegs {
        &self.fp_regs
    }

    pub fn fp_regs_mut(&mut self) -> &mut FpRegs {
        &mut self.fp_regs
    }
}

impl UserContextApiInternal for UserContext {
    fn execute(&mut self) -> crate::user::UserEvent {
        // set interrupt flag so that in user mode it can receive external interrupts
        // set ID flag which means cpu support CPUID instruction
        self.user_context.general.rflags |= (RFlags::INTERRUPT_FLAG | RFlags::ID).bits() as usize;

        const SYSCALL_TRAPNUM: u16 = 0x100;
        // return when it is syscall or cpu exception type is Fault or Trap.
        loop {
            self.user_context.run();
            match CpuException::to_cpu_exception(self.user_context.trap_num as u16) {
                Some(exception) => {
                    #[cfg(feature = "intel_tdx")]
                    if *exception == VIRTUALIZATION_EXCEPTION {
                        let ve_info =
                            tdcall::get_veinfo().expect("#VE handler: fail to get VE info\n");
                        handle_virtual_exception(self.general_regs_mut(), &ve_info);
                        continue;
                    }
                    if exception.typ == CpuExceptionType::FaultOrTrap
                        || exception.typ == CpuExceptionType::Fault
                        || exception.typ == CpuExceptionType::Trap
                    {
                        break;
                    }
                }
                None => {
                    if self.user_context.trap_num as u16 == SYSCALL_TRAPNUM {
                        break;
                    }
                }
            };
            call_irq_callback_functions(&self.as_trap_frame());
        }

        crate::arch::irq::enable_local();
        if self.user_context.trap_num as u16 != SYSCALL_TRAPNUM {
            self.cpu_exception_info = CpuExceptionInfo {
                page_fault_addr: unsafe { x86::controlregs::cr2() },
                id: self.user_context.trap_num,
                error_code: self.user_context.error_code,
            };
            UserEvent::Exception
        } else {
            UserEvent::Syscall
        }
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

/// As Osdev Wiki defines(https://wiki.osdev.org/Exceptions):
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
#[derive(PartialEq, Eq, Debug)]
pub enum CpuExceptionType {
    Fault,
    Trap,
    FaultOrTrap,
    Interrupt,
    Abort,
    Reserved,
}

#[derive(Debug, Eq, PartialEq)]
pub struct CpuException {
    pub number: u16,
    pub typ: CpuExceptionType,
}

/// Copy from: https://veykril.github.io/tlborm/decl-macros/building-blocks/counting.html#slice-length
macro_rules! replace_expr {
    ($_t:tt $sub:expr) => {
        $sub
    };
}

/// Copy from: https://veykril.github.io/tlborm/decl-macros/building-blocks/counting.html#slice-length
macro_rules! count_tts {
    ($($tts:tt)*) => {<[()]>::len(&[$(replace_expr!($tts ())),*])};
}

macro_rules! define_cpu_exception {
    ( $([ $name: ident = $exception_num:tt, $exception_type:tt]),* ) => {
        const EXCEPTION_LIST : [CpuException;count_tts!($($name)*)] = [
            $($name,)*
        ];
        $(
            pub const $name : CpuException = CpuException{
                number: $exception_num,
                typ: CpuExceptionType::$exception_type,
            };
        )*
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
    [INVAILD_TSS = 10, Fault],
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

impl CpuException {
    pub fn is_cpu_exception(trap_num: u16) -> bool {
        trap_num < EXCEPTION_LIST.len() as u16
    }

    pub fn to_cpu_exception(trap_num: u16) -> Option<&'static CpuException> {
        EXCEPTION_LIST.get(trap_num as usize)
    }
}

impl UserContextApi for UserContext {
    fn trap_number(&self) -> usize {
        self.user_context.trap_num
    }

    fn trap_error_code(&self) -> usize {
        self.user_context.error_code
    }

    fn syscall_num(&self) -> usize {
        self.rax()
    }

    fn syscall_ret(&self) -> usize {
        self.rax()
    }

    fn set_syscall_ret(&mut self, ret: usize) {
        self.set_rax(ret);
    }

    fn syscall_args(&self) -> [usize; 6] {
        [
            self.rdi(),
            self.rsi(),
            self.rdx(),
            self.r10(),
            self.r8(),
            self.r9(),
        ]
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
                #[inline(always)]
                pub fn $field(&self) -> usize {
                    self.user_context.general.$field
                }

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
    /// Create a new instance.
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

    /// Save the floating state given by a slice of u8.
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

    /// Clear the state of the instance.
    ///
    /// This method does not reset the underlying buffer that contains the floating
    /// point state; it only marks the buffer __invalid__.
    pub fn clear(&mut self) {
        self.is_valid = false;
    }

    /// Restore CPU's CPU floating pointer states from this instance.
    ///
    /// Panic. If the current state is invalid, the method will panic.
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
