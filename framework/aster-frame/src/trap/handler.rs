// SPDX-License-Identifier: MPL-2.0

use crate::{arch::irq::IRQ_LIST, cpu::CpuException};

#[cfg(feature = "intel_tdx")]
use crate::arch::tdx_guest::{handle_virtual_exception, TdxTrapFrame};
#[cfg(feature = "intel_tdx")]
use tdx_guest::tdcall;
use trapframe::TrapFrame;

#[cfg(feature = "intel_tdx")]
impl TdxTrapFrame for TrapFrame {
    fn rax(&self) -> usize {
        self.rax
    }
    fn set_rax(&mut self, rax: usize) {
        self.rax = rax;
    }
    fn rbx(&self) -> usize {
        self.rbx
    }
    fn set_rbx(&mut self, rbx: usize) {
        self.rbx = rbx;
    }
    fn rcx(&self) -> usize {
        self.rcx
    }
    fn set_rcx(&mut self, rcx: usize) {
        self.rcx = rcx;
    }
    fn rdx(&self) -> usize {
        self.rdx
    }
    fn set_rdx(&mut self, rdx: usize) {
        self.rdx = rdx;
    }
    fn rsi(&self) -> usize {
        self.rsi
    }
    fn set_rsi(&mut self, rsi: usize) {
        self.rsi = rsi;
    }
    fn rdi(&self) -> usize {
        self.rdi
    }
    fn set_rdi(&mut self, rdi: usize) {
        self.rdi = rdi;
    }
    fn rip(&self) -> usize {
        self.rip
    }
    fn set_rip(&mut self, rip: usize) {
        self.rip = rip;
    }
}

/// Only from kernel
#[no_mangle]
extern "sysv64" fn trap_handler(f: &mut TrapFrame) {
    if CpuException::is_cpu_exception(f.trap_num as u16) {
        #[cfg(feature = "intel_tdx")]
        if f.trap_num as u16 == 20 {
            let ve_info = tdcall::get_veinfo().expect("#VE handler: fail to get VE info\n");
            handle_virtual_exception(f, &ve_info);
            return;
        }
        panic!("cannot handle kernel cpu fault now, information:{:#x?}", f);
    } else {
        call_irq_callback_functions(f);
    }
}

pub(crate) fn call_irq_callback_functions(trap_frame: &TrapFrame) {
    let irq_line = IRQ_LIST.get().unwrap().get(trap_frame.trap_num).unwrap();
    let callback_functions = irq_line.callback_list();
    for callback_function in callback_functions.iter() {
        callback_function.call(trap_frame);
    }
    if !CpuException::is_cpu_exception(trap_frame.trap_num as u16) {
        crate::arch::interrupts_ack();
    }
}
