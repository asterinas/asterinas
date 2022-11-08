use kxos_frame::cpu::CpuContext;

use crate::{prelude::*, process::signal::signals::fault::FaultSignal};

/// We can't handle most exceptions, just send self a signal to force the process exit before return to user space.
pub fn handle_exception(context: &mut CpuContext) {
    let trap_info = context.trap_information.clone();
    let current = current!();
    let pid = current.pid();
    debug!("trap info = {:x?}", trap_info);
    debug!("cpu context = {:x?}", context);
    let signal = Box::new(FaultSignal::new(&trap_info));
    current.sig_queues().lock().enqueue(signal);
}
