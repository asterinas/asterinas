//! CPU.

use core::arch::x86_64::{_fxrstor, _fxsave};
use core::fmt::Debug;
use core::mem::MaybeUninit;

use crate::debug;
use crate::trap::{CalleeRegs, CallerRegs, SyscallFrame, TrapFrame};
use crate::vm::Pod;

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
    /// trap information, this field is all zero when it is syscall
    pub trap_information: TrapInformation,
}
#[derive(Clone, Default, Copy, Debug)]
#[repr(C)]
pub struct TrapInformation {
    pub cr2: u64,
    pub id: u64,
    pub err: u64,
    pub cs: u64,
    pub ss: u64,
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

impl From<SyscallFrame> for CpuContext {
    fn from(syscall: SyscallFrame) -> Self {
        Self {
            gp_regs: GpRegs {
                r8: syscall.caller.r8,
                r9: syscall.caller.r9,
                r10: syscall.caller.r10,
                r11: syscall.caller.r11,
                r12: syscall.callee.r12,
                r13: syscall.callee.r13,
                r14: syscall.callee.r14,
                r15: syscall.callee.r15,
                rdi: syscall.caller.rdi,
                rsi: syscall.caller.rsi,
                rbp: syscall.callee.rbp,
                rbx: syscall.callee.rbx,
                rdx: syscall.caller.rdx,
                rax: syscall.caller.rax,
                rcx: syscall.caller.rcx,
                rsp: syscall.callee.rsp,
                rip: syscall.caller.rcx,
                rflag: 0,
            },
            fs_base: 0,
            fp_regs: FpRegs::default(),
            trap_information: TrapInformation::default(),
        }
    }
}

impl Into<SyscallFrame> for CpuContext {
    fn into(self) -> SyscallFrame {
        SyscallFrame {
            caller: CallerRegs {
                rax: self.gp_regs.rax,
                rcx: self.gp_regs.rcx,
                rdx: self.gp_regs.rdx,
                rsi: self.gp_regs.rsi,
                rdi: self.gp_regs.rdi,
                r8: self.gp_regs.r8,
                r9: self.gp_regs.r9,
                r10: self.gp_regs.r10,
                r11: self.gp_regs.r11,
            },
            callee: CalleeRegs {
                rsp: self.gp_regs.rsp,
                rbx: self.gp_regs.rbx,
                rbp: self.gp_regs.rbp,
                r12: self.gp_regs.r12,
                r13: self.gp_regs.r13,
                r14: self.gp_regs.r14,
                r15: self.gp_regs.r15,
            },
        }
    }
}

impl From<TrapFrame> for CpuContext {
    fn from(trap: TrapFrame) -> Self {
        Self {
            gp_regs: GpRegs {
                r8: trap.caller.r8,
                r9: trap.caller.r9,
                r10: trap.caller.r10,
                r11: trap.caller.r11,
                r12: trap.callee.r12,
                r13: trap.callee.r13,
                r14: trap.callee.r14,
                r15: trap.callee.r15,
                rdi: trap.caller.rdi,
                rsi: trap.caller.rsi,
                rbp: trap.callee.rbp,
                rbx: trap.callee.rbx,
                rdx: trap.caller.rdx,
                rax: trap.caller.rax,
                rcx: trap.caller.rcx,
                rsp: trap.rsp,
                rip: trap.rip,
                rflag: trap.rflags,
            },
            fs_base: 0,
            fp_regs: FpRegs::default(),
            trap_information: TrapInformation {
                cr2: trap.cr2,
                id: trap.id,
                err: trap.err,
                cs: trap.cs,
                ss: trap.ss,
            },
        }
    }
}

impl Into<TrapFrame> for CpuContext {
    fn into(self) -> TrapFrame {
        let trap_information = self.trap_information;
        TrapFrame {
            caller: CallerRegs {
                rax: self.gp_regs.rax,
                rcx: self.gp_regs.rcx,
                rdx: self.gp_regs.rdx,
                rsi: self.gp_regs.rsi,
                rdi: self.gp_regs.rdi,
                r8: self.gp_regs.r8,
                r9: self.gp_regs.r9,
                r10: self.gp_regs.r10,
                r11: self.gp_regs.r11,
            },
            callee: CalleeRegs {
                rsp: self.gp_regs.rsp,
                rbx: self.gp_regs.rbx,
                rbp: self.gp_regs.rbp,
                r12: self.gp_regs.r12,
                r13: self.gp_regs.r13,
                r14: self.gp_regs.r14,
                r15: self.gp_regs.r15,
            },
            id: trap_information.id,
            err: trap_information.err,
            cr2: trap_information.cr2,
            rip: self.gp_regs.rip,
            cs: trap_information.cs,
            rflags: self.gp_regs.rflag,
            rsp: self.gp_regs.rsp,
            ss: trap_information.ss,
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
