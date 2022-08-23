#[derive(Debug, Default, Clone, Copy)]
#[repr(C)]
pub struct CallerRegs {
    pub rax: usize,
    pub rcx: usize,
    pub rdx: usize,
    pub rsi: usize,
    pub rdi: usize,
    pub r8: usize,
    pub r9: usize,
    pub r10: usize,
    pub r11: usize,
}

#[derive(Debug, Default, Clone, Copy)]
#[repr(C)]
pub struct CalleeRegs {
    pub rsp: usize,
    pub rbx: usize,
    pub rbp: usize,
    pub r12: usize,
    pub r13: usize,
    pub r14: usize,
    pub r15: usize,
}

#[derive(Debug, Default, Clone, Copy)]
#[repr(C)]
pub struct SyscallFrame {
    pub caller: CallerRegs,
    pub callee: CalleeRegs,
}

#[derive(Debug, Default, Clone, Copy)]
#[repr(C)]
pub struct TrapFrame {
    pub regs: CallerRegs,
    pub id: usize,
    pub err: usize,
    // Pushed by CPU
    pub rip: usize,
    pub cs: usize,
    pub rflags: usize,
    pub rsp: usize,
    pub ss: usize,
}
