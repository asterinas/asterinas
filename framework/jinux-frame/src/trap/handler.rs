use crate::{arch::irq::IRQ_LIST, cpu::CpuException};

#[cfg(feature = "intel_tdx")]
use tdx_guest::*;
use trapframe::TrapFrame;

#[cfg(feature = "intel_tdx")]
struct VeTrapFrame<'a>(&'a mut TrapFrame);

#[cfg(feature = "intel_tdx")]
impl TdxTrapFrame for VeTrapFrame<'_> {
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
}

/// Only from kernel
#[no_mangle]
extern "sysv64" fn trap_handler(f: &mut TrapFrame) {
    if CpuException::is_cpu_exception(f.trap_num as u16) {
        #[cfg(feature = "intel_tdx")]
        if f.trap_num as u16 == 20 {
            let ve_info = tdg_vp_veinfo_get().expect("#VE handler: fail to get VE info\n");
            let mut ve_f = VeTrapFrame(f);
            virtual_exception_handler(&mut ve_f, &ve_info);
        }
        #[cfg(not(feature = "intel_tdx"))]
        panic!("cannot handle kernel cpu fault now, information:{:#x?}", f);
    } else {
        call_irq_callback_functions(f);
    }
}

pub(crate) fn call_irq_callback_functions(trap_frame: &TrapFrame) {
    let irq_line = IRQ_LIST
        .get()
        .unwrap()
        .get(trap_frame.trap_num as usize)
        .unwrap();
    let callback_functions = irq_line.callback_list();
    for callback_function in callback_functions.iter() {
        callback_function.call(trap_frame);
    }
    if !CpuException::is_cpu_exception(trap_frame.trap_num as u16) {
        crate::arch::interrupts_ack();
    }
}
