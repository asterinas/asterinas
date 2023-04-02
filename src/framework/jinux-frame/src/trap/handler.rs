use crate::{arch::irq::IRQ_LIST, cpu::CpuException};

use trapframe::TrapFrame;

/// Only from kernel
#[no_mangle]
extern "sysv64" fn trap_handler(f: &mut TrapFrame) {
    if CpuException::is_cpu_exception(f.trap_num as u16) {
        panic!("cannot handle kernel cpu fault now, information:{:#x?}", f);
    }
    call_irq_callback_functions(f);
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
