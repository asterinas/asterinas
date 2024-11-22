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

/// Keyboard data register (R/W)
pub static KEYBOARD_DATA_PORT: IoPort<u8, ReadOnlyAccess> = unsafe { IoPort::new(0x60) };

/// Keyboard status register (R)
pub static KEYBOARD_STATUS_PORT: IoPort<u8, ReadOnlyAccess> = unsafe { IoPort::new(0x64) };

/// IrqLine for i8042 keyboard.
static KEYBOARD_IRQ: Once<SpinLock<IrqLine>> = Once::new();

pub(crate) fn init() {
    let irq = if !IO_APIC.is_completed() {
        pic::allocate_irq(1).unwrap()
    } else {
        let irq = IrqLine::alloc().unwrap();
        let mut io_apic = IO_APIC.get().unwrap().first().unwrap().lock();
        io_apic.enable(1, irq.clone()).unwrap();
        irq
    };

    KEYBOARD_IRQ.call_once(|| SpinLock::new(irq));
}

/// Registers a callback function to be called when there is keyboard input.
pub fn register_callback<F>(callback: F)
where
    F: Fn(&TrapFrame) + Sync + Send + 'static,
{
    let Some(irq) = KEYBOARD_IRQ.get() else {
        return;
    };

    irq.lock().on_active(callback);
}
