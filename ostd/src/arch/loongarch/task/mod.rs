// SPDX-License-Identifier: MPL-2.0

//! The architecture support of context switch.

use crate::task::TaskContextApi;

core::arch::global_asm!(include_str!("switch.S"));

#[derive(Debug, Default, Clone, Copy)]
#[repr(C)]
pub(crate) struct TaskContext {
    pub regs: CalleeRegs,
    pub ra: usize,
}

/// Callee-saved registers.
#[derive(Debug, Default, Clone, Copy)]
#[repr(C)]
pub struct CalleeRegs {
    /// sp
    pub sp: usize,
    /// fp
    pub fp: usize,
    /// s0
    pub s0: usize,
    /// s1
    pub s1: usize,
    /// s2
    pub s2: usize,
    /// s3
    pub s3: usize,
    /// s4
    pub s4: usize,
    /// s5
    pub s5: usize,
    /// s6
    pub s6: usize,
    /// s7
    pub s7: usize,
    /// s8
    pub s8: usize,
}

impl CalleeRegs {
    /// Creates new `CalleeRegs`
    pub const fn new() -> Self {
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

impl TaskContext {
    pub const fn new() -> Self {
        TaskContext {
            regs: CalleeRegs::new(),
            ra: 0,
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
