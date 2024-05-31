// SPDX-License-Identifier: MPL-2.0

use crate::task::TaskContextApi;

core::arch::global_asm!(include_str!("switch.S"));

#[derive(Debug, Default, Clone, Copy)]
#[repr(C)]
pub(crate) struct TaskContext {
    pub regs: CalleeRegs,
    pub rip: usize,
}

impl TaskContext {
    pub const fn new() -> Self {
        Self {
            regs: CalleeRegs::new(),
            rip: 0,
        }
    }
}

#[derive(Debug, Default, Clone, Copy)]
#[repr(C)]
pub struct CalleeRegs {
    pub rsp: u64,
    pub rbx: u64,
    pub rbp: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,
}

impl CalleeRegs {
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
