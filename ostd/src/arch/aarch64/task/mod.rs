// SPDX-License-Identifier: MPL-2.0

//! The architecture support of context switch.

use crate::task::TaskContextApi;

core::arch::global_asm!(include_str!("switch.S"));

/// ARM64 task context - callee-saved registers (x19-x30, sp, lr).
#[repr(C)]
#[derive(Clone, Debug)]
pub(crate) struct TaskContext {
    regs: CalleeRegs,
    lr: usize,
}

impl TaskContext {
    /// Creates a new `TaskContext`.
    pub(crate) const fn new() -> Self {
        TaskContext {
            regs: CalleeRegs::new(),
            lr: 0,
        }
    }
}

/// Callee-saved registers on ARM64: x19-x30, sp.
#[repr(C)]
#[derive(Clone, Debug)]
struct CalleeRegs {
    sp: u64,
    x19: u64,
    x20: u64,
    x21: u64,
    x22: u64,
    x23: u64,
    x24: u64,
    x25: u64,
    x26: u64,
    x27: u64,
    x28: u64,
    x29: u64, // Frame pointer
}

impl CalleeRegs {
    const fn new() -> Self {
        CalleeRegs {
            sp: 0,
            x19: 0,
            x20: 0,
            x21: 0,
            x22: 0,
            x23: 0,
            x24: 0,
            x25: 0,
            x26: 0,
            x27: 0,
            x28: 0,
            x29: 0,
        }
    }
}

impl TaskContextApi for TaskContext {
    fn set_instruction_pointer(&mut self, ip: usize) {
        self.lr = ip;
    }

    fn set_stack_pointer(&mut self, sp: usize) {
        self.regs.sp = sp as u64;
    }
}

unsafe extern "C" {
    pub(crate) unsafe fn context_switch(nxt: *const TaskContext, cur: *mut TaskContext);
    pub(crate) unsafe fn first_context_switch(nxt: *const TaskContext);
    pub(crate) unsafe fn kernel_task_entry_wrapper();
}
