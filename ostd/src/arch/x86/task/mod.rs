// SPDX-License-Identifier: MPL-2.0

//! The architecture support of context switch.

use crate::task::TaskContextApi;

core::arch::global_asm!(include_str!("switch.S"));

#[derive(Debug, Default, Clone, Copy)]
#[repr(C)]
pub(crate) struct TaskContext {
    pub regs: CalleeRegs,
    pub rip: usize,
    pub fsbase: usize,
}

impl TaskContext {
    pub const fn new() -> Self {
        Self {
            regs: CalleeRegs::new(),
            rip: 0,
            fsbase: 0,
        }
    }

    /// Sets thread-local storage pointer.
    pub fn set_tls_pointer(&mut self, tls: usize) {
        self.fsbase = tls;
    }

    /// Gets thread-local storage pointer.
    pub fn tls_pointer(&self) -> usize {
        self.fsbase
    }
}

/// Callee-saved registers.
#[derive(Debug, Default, Clone, Copy)]
#[repr(C)]
pub struct CalleeRegs {
    /// RSP
    pub rsp: u64,
    /// RBX
    pub rbx: u64,
    /// RBP
    pub rbp: u64,
    /// R12
    pub r12: u64,
    /// R13
    pub r13: u64,
    /// R14
    pub r14: u64,
    /// R15
    pub r15: u64,
}

impl CalleeRegs {
    /// Creates new `CalleeRegs`
    pub const fn new() -> Self {
        CalleeRegs {
            rsp: 0,
            rbx: 0,
            rbp: 0,
            r12: 0,
            r13: 0,
            r14: 0,
            r15: 0,
        }
    }
}

impl TaskContextApi for TaskContext {
    fn set_instruction_pointer(&mut self, ip: usize) {
        self.rip = ip;
    }

    fn instruction_pointer(&self) -> usize {
        self.rip
    }

    fn set_stack_pointer(&mut self, sp: usize) {
        self.regs.rsp = sp as u64;
    }

    fn stack_pointer(&self) -> usize {
        self.regs.rsp as usize
    }
}

extern "C" {
    pub(crate) fn context_switch(cur: *mut TaskContext, nxt: *const TaskContext);
}
