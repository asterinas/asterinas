use crate::{arch::irq::IRQ_LIST, cpu::CpuException};

#[cfg(feature = "intel_tdx")]
use crate::arch::tdx_guest::handle_virtual_exception;
#[cfg(feature = "intel_tdx")]
use tdx_guest::tdcall;
use trapframe::TrapFrame;

/// Only from kernel
#[no_mangle]
extern "sysv64" fn trap_handler(f: &mut TrapFrame) {
    if CpuException::is_cpu_exception(f.trap_num as u16) {
        #[cfg(feature = "intel_tdx")]
        if f.trap_num as u16 == 20 {
            let ve_info = tdcall::get_veinfo().expect("#VE handler: fail to get VE info\n");
            handle_virtual_exception(&mut (*f).into(), &ve_info);
        }
        #[cfg(not(feature = "intel_tdx"))]
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
