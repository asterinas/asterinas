// SPDX-License-Identifier: MPL-2.0

//! The console I/O.

use spin::Once;

use crate::{
    arch::{boot::DEVICE_TREE, mm::paddr_to_daddr},
    console::uart_ns16650a::{Ns16550aAccess, Ns16550aRegister, Ns16550aUart},
    sync::{LocalIrqDisabled, SpinLock},
};

/// The primary serial port, which serves as an early console.
pub static SERIAL_PORT: Once<SpinLock<Ns16550aUart<SerialAccess>, LocalIrqDisabled>> = Once::new();

/// Access to serial registers via I/O memory in LoongArch.
pub struct SerialAccess {
    base: *mut u8,
}

unsafe impl Send for SerialAccess {}
unsafe impl Sync for SerialAccess {}

impl SerialAccess {
    /// # Safety
    ///
    /// The caller must ensure that the base address is a valid serial base address and that it has
    /// exclusive ownership of the serial registers.
    const unsafe fn new(base: *mut u8) -> Self {
        Self { base }
    }
}

impl Ns16550aAccess for SerialAccess {
    fn read(&self, reg: Ns16550aRegister) -> u8 {
        // SAFETY: `self.base + reg` is a valid register of the serial port.
        unsafe { core::ptr::read_volatile(self.base.add(reg as u8 as usize)) }
    }

    fn write(&mut self, reg: Ns16550aRegister, val: u8) {
        // SAFETY: `self.base + reg` is a valid register of the serial port.
        unsafe { core::ptr::write_volatile(self.base.add(reg as u8 as usize), val) };
    }
}

/// Initializes the serial port.
pub(crate) fn init() {
    let Some(base_address) = lookup_uart_base_address() else {
        return;
    };

    // SAFETY:
    // 1. The base address is valid and correct because it is acquired from the device tree and
    //    mapped in DMW2.
    // 2. FIXME: We should reserve the address region in `io_mem_allocator` to ensure the
    //    exclusive ownership.
    let access = unsafe { SerialAccess::new(paddr_to_daddr(base_address) as *mut u8) };
    let mut serial = Ns16550aUart::new(access);
    serial.init();
    SERIAL_PORT.call_once(move || SpinLock::new(serial));
}

fn lookup_uart_base_address() -> Option<usize> {
    let device_tree = DEVICE_TREE.get().unwrap();
    let stdout_path = device_tree
        .find_node("/chosen")?
        .property("stdout-path")?
        .as_str()?;
    let stdout = device_tree.find_node(stdout_path)?;
    if stdout.compatible()?.all().any(|c| c == "ns16550a") {
        Some(stdout.reg()?.next()?.starting_address as usize)
    } else {
        None
    }
}
