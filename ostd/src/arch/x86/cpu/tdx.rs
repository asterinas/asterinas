// SPDX-License-Identifier: MPL-2.0

use tdx_guest::{handle_virtual_exception as do_handle_virtual_exception, tdcall, TdxTrapFrame};

use crate::cpu::{RawGeneralRegs, UserContext};

pub(crate) fn handle_virtualization_exception(user_context: &mut UserContext) {
    let ve_info = tdcall::get_veinfo().expect("#VE handler: fail to get VE info\n");
    let mut generalrags_wrapper = GeneralRegsWrapper(&mut *user_context.general_regs_mut());
    do_handle_virtual_exception(&mut generalrags_wrapper, &ve_info);
    *user_context.general_regs_mut() = *generalrags_wrapper.0;
}

struct GeneralRegsWrapper<'a>(&'a mut RawGeneralRegs);

impl TdxTrapFrame for GeneralRegsWrapper<'_> {
    fn rax(&self) -> usize {
        self.0.rax
    }
    fn set_rax(&mut self, rax: usize) {
        self.0.rax = rax;
    }
    fn rbx(&self) -> usize {
        self.0.rbx
    }
    fn set_rbx(&mut self, rbx: usize) {
        self.0.rbx = rbx;
    }
    fn rcx(&self) -> usize {
        self.0.rcx
    }
    fn set_rcx(&mut self, rcx: usize) {
        self.0.rcx = rcx;
    }
    fn rdx(&self) -> usize {
        self.0.rdx
    }
    fn set_rdx(&mut self, rdx: usize) {
        self.0.rdx = rdx;
    }
    fn rsi(&self) -> usize {
        self.0.rsi
    }
    fn set_rsi(&mut self, rsi: usize) {
        self.0.rsi = rsi;
    }
    fn rdi(&self) -> usize {
        self.0.rdi
    }
    fn set_rdi(&mut self, rdi: usize) {
        self.0.rdi = rdi;
    }
    fn rip(&self) -> usize {
        self.0.rip
    }
    fn set_rip(&mut self, rip: usize) {
        self.0.rip = rip;
    }
    fn r8(&self) -> usize {
        self.0.r8
    }
    fn set_r8(&mut self, r8: usize) {
        self.0.r8 = r8;
    }
    fn r9(&self) -> usize {
        self.0.r9
    }
    fn set_r9(&mut self, r9: usize) {
        self.0.r9 = r9;
    }
    fn r10(&self) -> usize {
        self.0.r10
    }
    fn set_r10(&mut self, r10: usize) {
        self.0.r10 = r10;
    }
    fn r11(&self) -> usize {
        self.0.r11
    }
    fn set_r11(&mut self, r11: usize) {
        self.0.r11 = r11;
    }
    fn r12(&self) -> usize {
        self.0.r12
    }
    fn set_r12(&mut self, r12: usize) {
        self.0.r12 = r12;
    }
    fn r13(&self) -> usize {
        self.0.r13
    }
    fn set_r13(&mut self, r13: usize) {
        self.0.r13 = r13;
    }
    fn r14(&self) -> usize {
        self.0.r14
    }
    fn set_r14(&mut self, r14: usize) {
        self.0.r14 = r14;
    }
    fn r15(&self) -> usize {
        self.0.r15
    }
    fn set_r15(&mut self, r15: usize) {
        self.0.r15 = r15;
    }
    fn rbp(&self) -> usize {
        self.0.rbp
    }
    fn set_rbp(&mut self, rbp: usize) {
        self.0.rbp = rbp;
    }
}
