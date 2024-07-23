// SPDX-License-Identifier: MPL-2.0

//! CPU.

use alloc::vec::Vec;
use core::fmt::Debug;

use bitvec::{
    prelude::{BitVec, Lsb0},
    slice::IterOnes,
};
use log::debug;
use riscv::register::scause::{Exception, Trap};
pub use trapframe::{GeneralRegs as RawGeneralRegs, UserContext as RawUserContext};

use crate::user::{ReturnReason, UserContextApi, UserContextApiInternal};

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
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct UserContext {
    pub user_context: RawUserContext,
    trap: Trap,
    fp_regs: FpRegs,
    cpu_exception_info: CpuExceptionInfo,
}

#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct CpuExceptionInfo {
    pub code: Exception,
    pub page_fault_addr: usize,
}

impl Default for UserContext {
    fn default() -> Self {
        let mut user_context = RawUserContext::default();
        // Set FS to Clean to enable floating point instructions
        user_context.sstatus |= (riscv::register::sstatus::FS::Clean as usize) << 13;

        UserContext {
            user_context,
            trap: Trap::Exception(Exception::Unknown),
            fp_regs: FpRegs::default(),
            cpu_exception_info: CpuExceptionInfo::default(),
        }
    }
}

impl Default for CpuExceptionInfo {
    fn default() -> Self {
        CpuExceptionInfo {
            code: Exception::Unknown,
            page_fault_addr: 0,
        }
    }
}

impl CpuExceptionInfo {
    pub fn cpu_exception(&self) -> &CpuException {
        &self.code
    }
}

impl UserContext {
    pub fn general_regs(&self) -> &RawGeneralRegs {
        &self.user_context.general
    }

    pub fn general_regs_mut(&mut self) -> &mut RawGeneralRegs {
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
    fn execute<F>(&mut self, mut has_kernel_event: F) -> ReturnReason
    where
        F: FnMut() -> bool,
    {
        let ret = loop {
            self.user_context.run();
            use riscv::register::scause::Interrupt::*;
            let scause = riscv::register::scause::read();
            match scause.cause() {
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

    fn as_trap_frame(&self) -> trapframe::TrapFrame {
        trapframe::TrapFrame {
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

    fn set_instruction_pointer(&mut self, ip: usize) {
        self.user_context.set_ip(ip);
    }

    fn set_stack_pointer(&mut self, sp: usize) {
        self.user_context.set_sp(sp);
    }

    fn stack_pointer(&self) -> usize {
        self.user_context.get_sp()
    }

    fn instruction_pointer(&self) -> usize {
        self.user_context.sepc
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

/// The floating-point state of CPU.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct FpRegs {
    buf: [u8; 8 * 32],
    fcsr: u32,
    is_valid: bool,
}

core::arch::global_asm!(include_str!("fpu.S"));

extern "C" {
    fn fstate_save(buf: &mut FpRegs);
    fn fstate_restore(buf: &FpRegs);
}

impl FpRegs {
    /// Create a new instance.
    ///
    /// Note that a newly-created instance's floating point state is not
    /// initialized, thus considered invalid (i.e., `self.is_valid() == false`).
    pub fn new() -> Self {
        // The buffer address requires 16bytes alignment.
        Self {
            buf: [0; 8 * 32],
            fcsr: 0,
            is_valid: false,
        }
    }

    /// Save CPU's current floating pointer states into this instance.
    pub fn save(&mut self) {
        debug!("save fpregs");
        unsafe {
            fstate_save(self);
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
        self.buf.copy_from_slice(src);
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
        unsafe {
            fstate_restore(self);
        }
        debug!("restore fpregs success");
    }

    /// Returns the floating point state as a slice.
    ///
    /// Note that the slice may contain garbage if `self.is_valid() == false`.
    pub fn as_slice(&self) -> &[u8] {
        &self.buf
    }
}

impl Default for FpRegs {
    fn default() -> Self {
        Self::new()
    }
}

pub type CpuException = Exception;

/// Sets the base address for the CPU local storage by writing to the `tp` register.
/// This operation is marked as `unsafe` because it directly interfaces with low-level CPU registers.
///
/// # Safety
///
///  - This function is safe to call provided that the `tp` registe is dedicated entirely for CPU local storage
///    and is not concurrently accessed for other purposes.
///  - The caller must ensure that `addr` is a valid address and properly aligned, as required by the CPU.
///  - This function should only be called in contexts where the CPU is in a state to accept such changes,
///    such as during processor initialization.
pub(crate) unsafe fn set_cpu_local_base(addr: u64) {
    core::arch::asm!(
        "mv tp, {addr}",
        addr = in(reg) addr,
    );
}

/// Gets the base address for the CPU local storage by reading the FS base model-specific register.
pub(crate) fn get_cpu_local_base() -> u64 {
    let mut tp;
    unsafe {
        core::arch::asm!(
            "mv {tp}, tp",
            tp = out(reg) tp,
        );
    }
    tp
}
