// SPDX-License-Identifier: MPL-2.0

//! Provides i8042 PC keyboard I/O port access.

use spin::Once;

use crate::{
    arch::x86::{
        device::io_port::{IoPort, ReadOnlyAccess},
        kernel::{pic, IO_APIC},
    },
    sync::SpinLock,
    trap::{IrqLine, TrapFrame},
};

/// Data register (R/W)
pub static DATA_PORT: IoPort<u8, ReadOnlyAccess> = unsafe { IoPort::new(0x60) };

/// Status register (R)
pub static STATUS_PORT: IoPort<u8, ReadOnlyAccess> = unsafe { IoPort::new(0x64) };

/// IrqLine for i8042 keyboard.
static IRQ_LINE: Once<SpinLock<IrqLine>> = Once::new();

pub(crate) fn init() {
    let irq = if !IO_APIC.is_completed() {
        pic::allocate_irq(1).unwrap()
    } else {
        let irq = IrqLine::alloc().unwrap();
        let mut io_apic = IO_APIC.get().unwrap().first().unwrap().lock();
        io_apic.enable(1, irq.clone()).unwrap();
        irq
    };

    IRQ_LINE.call_once(|| SpinLock::new(irq));
}

/// Registers a callback function to be called when there is keyboard input.
pub fn register_callback<F>(callback: F)
where
    F: Fn(&TrapFrame) + Sync + Send + 'static,
{
    let Some(irq) = IRQ_LINE.get() else {
        return;
    };

    irq.disable_irq().lock().on_active(callback);
}
