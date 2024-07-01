// SPDX-License-Identifier: MPL-2.0

use ostd::{
    cpu::{RawGeneralRegs, UserContext},
    Pod,
};

use crate::cpu::LinuxAbi;

impl LinuxAbi for UserContext {
    fn syscall_num(&self) -> usize {
        self.rax()
    }

    fn syscall_ret(&self) -> usize {
        self.rax()
    }

    fn set_syscall_ret(&mut self, ret: usize) {
        self.set_rax(ret);
    }

    fn syscall_args(&self) -> [usize; 6] {
        [
            self.rdi(),
            self.rsi(),
            self.rdx(),
            self.r10(),
            self.r8(),
            self.r9(),
        ]
    }

    fn set_tls_pointer(&mut self, tls: usize) {
        self.set_fsbase(tls);
    }

    fn tls_pointer(&self) -> usize {
        self.fsbase()
    }
}

/// General-purpose registers.
#[derive(Debug, Clone, Copy, Pod, Default)]
#[repr(C)]
pub struct GpRegs {
    pub rax: usize,
    pub rbx: usize,
    pub rcx: usize,
    pub rdx: usize,
    pub rsi: usize,
    pub rdi: usize,
    pub rbp: usize,
    pub rsp: usize,
    pub r8: usize,
    pub r9: usize,
    pub r10: usize,
    pub r11: usize,
    pub r12: usize,
    pub r13: usize,
    pub r14: usize,
    pub r15: usize,
    pub rip: usize,
    pub rflags: usize,
    pub fsbase: usize,
    pub gsbase: usize,
}

macro_rules! copy_gp_regs {
    ($src: ident, $dst: ident) => {
        $dst.rax = $src.rax;
        $dst.rbx = $src.rax;
        $dst.rcx = $src.rcx;
        $dst.rdx = $src.rdx;
        $dst.rsi = $src.rsi;
        $dst.rdi = $src.rdi;
        $dst.rbp = $src.rbp;
        $dst.rsp = $src.rsp;
        $dst.r8 = $src.r8;
        $dst.r9 = $src.r9;
        $dst.r10 = $src.r10;
        $dst.r11 = $src.r11;
        $dst.r12 = $src.r12;
        $dst.r13 = $src.r13;
        $dst.r14 = $src.r14;
        $dst.r15 = $src.r15;
        $dst.rip = $src.rip;
        $dst.rflags = $src.rflags;
        $dst.fsbase = $src.fsbase;
        $dst.gsbase = $src.gsbase;
    };
}

impl GpRegs {
    pub fn copy_to_raw(&self, dst: &mut RawGeneralRegs) {
        copy_gp_regs!(self, dst);
    }

    pub fn copy_from_raw(&mut self, src: &RawGeneralRegs) {
        copy_gp_regs!(src, self);
    }
}
