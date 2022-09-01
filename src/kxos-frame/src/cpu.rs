//! CPU.

use crate::trap::{CalleeRegs, CallerRegs, SyscallFrame, TrapFrame};

/// Defines a CPU-local variable.
#[macro_export]
macro_rules! cpu_local {
    () => {
        todo!()
    };
}

/// Returns the number of CPUs.
pub fn num_cpus() -> u32 {
    todo!()
}

/// Returns the ID of this CPU.
pub fn this_cpu() -> u32 {
    todo!()
}

/// Cpu context, including both general-purpose registers and floating-point registers.
#[derive(Clone, Default, Copy)]
#[repr(C)]
pub struct CpuContext {
    pub gp_regs: GpRegs,
    pub fs_base: u64,
    pub fp_regs: FpRegs,
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

impl From<SyscallFrame> for CpuContext {
    fn from(syscall: SyscallFrame) -> Self {
        Self {
            gp_regs: GpRegs {
                r8: syscall.caller.r8 as u64,
                r9: syscall.caller.r9 as u64,
                r10: syscall.caller.r10 as u64,
                r11: syscall.caller.r11 as u64,
                r12: syscall.callee.r12 as u64,
                r13: syscall.callee.r13 as u64,
                r14: syscall.callee.r14 as u64,
                r15: syscall.callee.r15 as u64,
                rdi: syscall.caller.rdi as u64,
                rsi: syscall.caller.rsi as u64,
                rbp: syscall.callee.rbp as u64,
                rbx: syscall.callee.rbx as u64,
                rdx: syscall.caller.rdx as u64,
                rax: syscall.caller.rax as u64,
                rcx: syscall.caller.rcx as u64,
                rsp: syscall.callee.rsp as u64,
                rip: 0,
                rflag: 0,
            },
            fs_base: 0,
            fp_regs: FpRegs::default(),
        }
    }
}

impl Into<SyscallFrame> for CpuContext {
    fn into(self) -> SyscallFrame {
        SyscallFrame {
            caller: CallerRegs {
                rax: self.gp_regs.rax as usize,
                rcx: self.gp_regs.rcx as usize,
                rdx: self.gp_regs.rdx as usize,
                rsi: self.gp_regs.rsi as usize,
                rdi: self.gp_regs.rdi as usize,
                r8: self.gp_regs.r8 as usize,
                r9: self.gp_regs.r9 as usize,
                r10: self.gp_regs.r10 as usize,
                r11: self.gp_regs.r11 as usize,
            },
            callee: CalleeRegs {
                rsp: self.gp_regs.rsp as usize,
                rbx: self.gp_regs.rbx as usize,
                rbp: self.gp_regs.rbp as usize,
                r12: self.gp_regs.r12 as usize,
                r13: self.gp_regs.r13 as usize,
                r14: self.gp_regs.r14 as usize,
                r15: self.gp_regs.r15 as usize,
            },
        }
    }
}

impl From<TrapFrame> for CpuContext {
    fn from(trap: TrapFrame) -> Self {
        Self {
            gp_regs: GpRegs {
                r8: trap.regs.r8 as u64,
                r9: trap.regs.r9 as u64,
                r10: trap.regs.r10 as u64,
                r11: trap.regs.r11 as u64,
                r12: trap.id as u64,
                r13: trap.err as u64,
                r14: trap.cs as u64,
                r15: trap.ss as u64,
                rdi: trap.regs.rdi as u64,
                rsi: trap.regs.rsi as u64,
                rbp: 0 as u64,
                rbx: 0 as u64,
                rdx: trap.regs.rdx as u64,
                rax: trap.regs.rax as u64,
                rcx: trap.regs.rcx as u64,
                rsp: trap.rsp as u64,
                rip: trap.rip as u64,
                rflag: trap.rflags as u64,
            },
            fs_base: 0,
            fp_regs: FpRegs::default(),
        }
    }
}

impl Into<TrapFrame> for CpuContext {
    fn into(self) -> TrapFrame {
        TrapFrame {
            regs: CallerRegs {
                rax: self.gp_regs.rax as usize,
                rcx: self.gp_regs.rcx as usize,
                rdx: self.gp_regs.rdx as usize,
                rsi: self.gp_regs.rsi as usize,
                rdi: self.gp_regs.rdi as usize,
                r8: self.gp_regs.r8 as usize,
                r9: self.gp_regs.r9 as usize,
                r10: self.gp_regs.r10 as usize,
                r11: self.gp_regs.r11 as usize,
            },
            id: self.gp_regs.r12 as usize,
            err: self.gp_regs.r13 as usize,
            rip: self.gp_regs.rip as usize,
            cs: self.gp_regs.r14 as usize,
            rflags: self.gp_regs.rflag as usize,
            rsp: self.gp_regs.rsp as usize,
            ss: self.gp_regs.r15 as usize,
        }
    }
}

/// The floating-point state of CPU.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct FpRegs {
    //buf: Aligned<A16, [u8; 512]>,
    is_valid: bool,
}

impl FpRegs {
    /// Create a new instance.
    ///
    /// Note that a newly-created instance's floating point state is not
    /// initialized, thus considered invalid (i.e., `self.is_valid() == false`).
    pub fn new() -> Self {
        //let buf = Aligned(unsafe { MaybeUninit::uninit().assume_init() });
        //let is_valid = false;
        //Self { buf, is_valid }
        Self { is_valid: false }
        // todo!("import aligned")
    }

    /// Save CPU's current floating pointer states into this instance.
    pub fn save(&mut self) {
        // unsafe {
        //     _fxsave(self.buf.as_mut_ptr() as *mut u8);
        // }
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
        //(&mut self.buf).copy_from_slice(src);
        //self.is_valid = true;
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
        assert!(self.is_valid);
        //unsafe { _fxrstor(self.buf.as_ptr()) };
    }

    /// Returns the floating point state as a slice.
    ///
    /// Note that the slice may contain garbage if `self.is_valid() == false`.
    pub fn as_slice(&self) -> &[u8] {
        //&*self.buf
        todo!()
    }
}

impl Default for FpRegs {
    fn default() -> Self {
        Self::new()
    }
}
