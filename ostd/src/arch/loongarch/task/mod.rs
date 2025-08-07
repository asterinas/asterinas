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
    sp: usize,
    fp: usize,
    s0: usize,
    s1: usize,
    s2: usize,
    s3: usize,
    s4: usize,
    s5: usize,
    s6: usize,
    s7: usize,
    s8: usize,
}

impl CalleeRegs {
    /// Creates a new `CalleeRegs`.
    pub(self) const fn new() -> Self {
        CalleeRegs {
            sp: 0,
            fp: 0,
            s0: 0,
            s1: 0,
            s2: 0,
            s3: 0,
            s4: 0,
            s5: 0,
            s6: 0,
            s7: 0,
            s8: 0,
        }
    }
}

impl TaskContextApi for TaskContext {
    fn set_instruction_pointer(&mut self, ip: usize) {
        self.ra = ip;
    }

    fn set_stack_pointer(&mut self, sp: usize) {
        self.regs.sp = sp;
    }
}

unsafe extern "C" {
    pub(crate) unsafe fn context_switch(cur: *mut TaskContext, nxt: *const TaskContext);
}
