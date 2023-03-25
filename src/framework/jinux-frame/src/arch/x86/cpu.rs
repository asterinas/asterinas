//! CPU.

use core::arch::x86_64::{_fxrstor, _fxsave};
use core::fmt::Debug;
use core::mem::MaybeUninit;

use trapframe::{GeneralRegs, UserContext};

use log::debug;
use pod::Pod;

/// Defines a CPU-local variable.
#[macro_export]
macro_rules! cpu_local {
    () => {
        todo!()
    };
}

/// Returns the number of CPUs.
pub fn num_cpus() -> u32 {
    // FIXME: we only start one cpu now.
    1
}

/// Returns the ID of this CPU.
pub fn this_cpu() -> u32 {
    todo!()
}

/// Cpu context, including both general-purpose registers and floating-point registers.
#[derive(Clone, Default, Copy, Debug)]
#[repr(C)]
pub struct CpuContext {
    pub fp_regs: FpRegs,
    pub gp_regs: GpRegs,
    pub fs_base: u64,
    pub gs_base: u64,
    /// trap information, this field is all zero when it is syscall
    pub trap_information: TrapInformation,
}

impl CpuContext {
    pub fn set_rax(&mut self, rax: u64) {
        self.gp_regs.rax = rax;
    }

    pub fn set_rsp(&mut self, rsp: u64) {
        self.gp_regs.rsp = rsp;
    }

    pub fn set_rip(&mut self, rip: u64) {
        self.gp_regs.rip = rip;
    }

    pub fn set_fsbase(&mut self, fs_base: u64) {
        self.fs_base = fs_base;
    }
}

#[derive(Clone, Default, Copy, Debug)]
#[repr(C)]
pub struct TrapInformation {
    pub cr2: u64,
    pub id: u64,
    pub err: u64,
}

/// The general-purpose registers of CPU.
#[derive(Clone, Copy, Debug, Default)]
#[repr(C)]
pub struct GpRegs {
    pub r8: u64,
    pub r9: u64,
    pub r10: u64,
    pub r11: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,
    pub rdi: u64,
    pub rsi: u64,
    pub rbp: u64,
    pub rbx: u64,
    pub rdx: u64,
    pub rax: u64,
    pub rcx: u64,
    pub rsp: u64,
    pub rip: u64,
    pub rflag: u64,
}

unsafe impl Pod for GpRegs {}
unsafe impl Pod for TrapInformation {}
unsafe impl Pod for CpuContext {}
unsafe impl Pod for FpRegs {}

impl From<UserContext> for CpuContext {
    fn from(value: UserContext) -> Self {
        Self {
            gp_regs: GpRegs {
                r8: value.general.r8 as u64,
                r9: value.general.r9 as u64,
                r10: value.general.r10 as u64,
                r11: value.general.r11 as u64,
                r12: value.general.r12 as u64,
                r13: value.general.r13 as u64,
                r14: value.general.r14 as u64,
                r15: value.general.r15 as u64,
                rdi: value.general.rdi as u64,
                rsi: value.general.rsi as u64,
                rbp: value.general.rbp as u64,
                rbx: value.general.rbx as u64,
                rdx: value.general.rdx as u64,
                rax: value.general.rax as u64,
                rcx: value.general.rcx as u64,
                rsp: value.general.rsp as u64,
                rip: value.general.rip as u64,
                rflag: value.general.rflags as u64,
            },
            fs_base: value.general.fsbase as u64,
            fp_regs: FpRegs::default(),
            trap_information: TrapInformation {
                cr2: x86_64::registers::control::Cr2::read_raw(),
                id: value.trap_num as u64,
                err: value.error_code as u64,
            },
            gs_base: value.general.gsbase as u64,
        }
    }
}

impl Into<UserContext> for CpuContext {
    fn into(self) -> UserContext {
        UserContext {
            trap_num: self.trap_information.id as usize,
            error_code: self.trap_information.err as usize,
            general: GeneralRegs {
                rax: self.gp_regs.rax as usize,
                rbx: self.gp_regs.rbx as usize,
                rcx: self.gp_regs.rcx as usize,
                rdx: self.gp_regs.rdx as usize,
                rsi: self.gp_regs.rsi as usize,
                rdi: self.gp_regs.rdi as usize,
                rbp: self.gp_regs.rbp as usize,
                rsp: self.gp_regs.rsp as usize,
                r8: self.gp_regs.r8 as usize,
                r9: self.gp_regs.r9 as usize,
                r10: self.gp_regs.r10 as usize,
                r11: self.gp_regs.r11 as usize,
                r12: self.gp_regs.r12 as usize,
                r13: self.gp_regs.r13 as usize,
                r14: self.gp_regs.r14 as usize,
                r15: self.gp_regs.r15 as usize,
                rip: self.gp_regs.rip as usize,
                rflags: self.gp_regs.rflag as usize,
                fsbase: self.fs_base as usize,
                gsbase: self.gs_base as usize,
            },
        }
    }
}

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
            buf: unsafe { MaybeUninit::uninit().assume_init() },
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
            _fxsave((&mut self.buf.data).as_mut_ptr() as *mut u8);
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
        (&mut self.buf.data).copy_from_slice(src);
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
        unsafe { _fxrstor((&self.buf.data).as_ptr()) };
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
