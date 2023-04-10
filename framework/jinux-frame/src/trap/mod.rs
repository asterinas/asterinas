mod handler;
mod irq;

pub(crate) use self::handler::call_irq_callback_functions;
pub use self::irq::{allocate_irq, disable_local, DisabledLocalIrqGuard, IrqAllocateHandle};
pub(crate) use self::irq::{allocate_target_irq, IrqCallbackHandle, IrqLine};
pub use trapframe::TrapFrame;

pub(crate) fn init() {
    unsafe {
        trapframe::init();
    }
}
