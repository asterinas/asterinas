// SPDX-License-Identifier: MPL-2.0

//! The architecture support of context switch.

use crate::task::TaskContextApi;

core::arch::global_asm!(include_str!("switch.S"));

#[derive(Debug, Clone)]
#[repr(C)]
pub(crate) struct TaskContext {
    regs: CalleeRegs,
    rip: usize,
    fsbase: usize,
}

impl TaskContext {
    pub(crate) const fn new() -> Self {
        Self {
            regs: CalleeRegs::new(),
            rip: 0,
            fsbase: 0,
        }
    }
}

/// Callee-saved registers.
#[derive(Debug, Clone)]
#[repr(C)]
struct CalleeRegs {
    rsp: u64,
    rbx: u64,
    rbp: u64,
    r12: u64,
    r13: u64,
    r14: u64,
    r15: u64,
}

impl CalleeRegs {
    /// Creates new `CalleeRegs`
    pub(self) const fn new() -> Self {
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

    fn set_stack_pointer(&mut self, sp: usize) {
        self.regs.rsp = sp as u64;
    }
}

extern "C" {
    pub(crate) fn context_switch(cur: *mut TaskContext, nxt: *const TaskContext);
}
