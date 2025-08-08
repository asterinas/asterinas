// SPDX-License-Identifier: MPL-2.0

//! The architecture support of context switch.

use crate::task::TaskContextApi;

core::arch::global_asm!(include_str!("switch.S"));

#[derive(Debug, Clone)]
#[repr(C)]
pub(crate) struct TaskContext {
    regs: CalleeRegs,
    ra: usize,
}

impl TaskContext {
    /// Creates a new `TaskContext`.
    pub(crate) const fn new() -> Self {
        TaskContext {
            regs: CalleeRegs::new(),
            ra: 0,
        }
    }
}

/// Callee-saved registers.
#[derive(Debug, Clone)]
#[repr(C)]
struct CalleeRegs {
    sp: u64,
    s0: u64,
    s1: u64,
    s2: u64,
    s3: u64,
    s4: u64,
    s5: u64,
    s6: u64,
    s7: u64,
    s8: u64,
    s9: u64,
    s10: u64,
    s11: u64,
}

impl CalleeRegs {
    /// Creates a new `CalleeRegs`.
    pub(self) const fn new() -> Self {
        CalleeRegs {
            sp: 0,
            s0: 0,
            s1: 0,
            s2: 0,
            s3: 0,
            s4: 0,
            s5: 0,
            s6: 0,
            s7: 0,
            s8: 0,
            s9: 0,
            s10: 0,
            s11: 0,
        }
    }
}

impl TaskContextApi for TaskContext {
    fn set_instruction_pointer(&mut self, ip: usize) {
        self.ra = ip;
    }

    fn set_stack_pointer(&mut self, sp: usize) {
        self.regs.sp = sp as u64;
    }
}

extern "C" {
    pub(crate) fn context_switch(cur: *mut TaskContext, nxt: *const TaskContext);
}
