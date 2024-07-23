use crate::task::TaskContextApi;

core::arch::global_asm!(include_str!("switch.S"));

#[derive(Debug, Default, Clone, Copy)]
#[repr(C)]
pub(crate) struct TaskContext {
    pub regs: CalleeRegs,
    pub pc: usize,
}

#[derive(Debug, Default, Clone, Copy)]
#[repr(C)]
pub struct CalleeRegs {
    pub sp: u64,
    pub s0: u64,
    pub s1: u64,
    pub s2: u64,
    pub s3: u64,
    pub s4: u64,
    pub s5: u64,
    pub s6: u64,
    pub s7: u64,
    pub s8: u64,
    pub s9: u64,
    pub s10: u64,
    pub s11: u64,
}

impl CalleeRegs {
    pub const fn new() -> Self {
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

impl TaskContext {
    pub const fn new() -> Self {
        TaskContext {
            regs: CalleeRegs::new(),
            pc: 0,
        }
    }
}

impl TaskContextApi for TaskContext {
    fn set_instruction_pointer(&mut self, ip: usize) {
        self.pc = ip;
    }

    fn instruction_pointer(&self) -> usize {
        self.pc
    }

    fn set_stack_pointer(&mut self, sp: usize) {
        self.regs.sp = sp as u64;
    }

    fn stack_pointer(&self) -> usize {
        self.regs.sp as usize
    }
}

extern "C" {
    pub(crate) fn context_switch(cur: *mut TaskContext, nxt: *const TaskContext);
}
