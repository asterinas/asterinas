use jinux_frame::{
    cpu::{CpuContext, TrapInformation},
    trap::PAGE_FAULT,
};

use crate::{prelude::*, process::signal::signals::fault::FaultSignal};

/// We can't handle most exceptions, just send self a fault signal before return to user space.
pub fn handle_exception(context: &mut CpuContext) {
    let trap_info = context.trap_information.clone();
    debug!("trap info = {:x?}", trap_info);
    match trap_info.id {
        PAGE_FAULT => handle_page_fault(&trap_info),
        _ => {
            // We current do nothing about other exceptions
            generate_fault_signal(&trap_info);
        }
    }
}

fn handle_page_fault(trap_info: &TrapInformation) {
    const PAGE_NOT_PRESENT_ERROR_MASK: u64 = 0x1 << 0;
    const WRITE_ACCESS_MASK: u64 = 0x1 << 1;
    if trap_info.err & PAGE_NOT_PRESENT_ERROR_MASK == 0 {
        // TODO: If page is not present, we should ask the vmar try to commit this page
        generate_fault_signal(trap_info)
    } else {
        // Otherwise, the page fault is caused by page protection error.
        generate_fault_signal(trap_info)
    }
}

/// generate a fault signal for current process.
fn generate_fault_signal(trap_info: &TrapInformation) {
    let current = current!();
    let signal = Box::new(FaultSignal::new(trap_info));
    current.sig_queues().lock().enqueue(signal);
}
