// SPDX-License-Identifier: MPL-2.0

use crate::{
    arch::device::io_port::{ReadWriteAccess, WriteOnlyAccess},
    info,
    io::{IoPort, sensitive_io_port},
};

sensitive_io_port! {
    unsafe {
        static MASTER_CMD: IoPort<u8, WriteOnlyAccess> = IoPort::new(0x20);
        static MASTER_DATA: IoPort<u8, ReadWriteAccess> = IoPort::new(0x21);
        static SLAVE_CMD: IoPort<u8, WriteOnlyAccess> = IoPort::new(0xA0);
        static SLAVE_DATA: IoPort<u8, WriteOnlyAccess> = IoPort::new(0xA1);
    }
}

const IRQ_OFFSET: u8 = 0x20;

/// Initializes and disables the 8259 Programmable Interrupt Controller (PIC).
pub fn init_and_disable() {
    info!("Initializing PIC as disabled");

    set_mask(0xff, 0xff);
}

/// Detects whether a legacy 8259A PIC is present via I/O port probing.
///
/// Reference: <https://elixir.bootlin.com/linux/v7.0/source/arch/x86/kernel/i8259.c#L306>
pub(super) fn is_legacy_present() -> bool {
    // Mask all except cascade (IRQ 2) on master IMR.
    const PROBE_VAL: u8 = 0xFB;

    // Mask slave PIC first to avoid interrupts during probe.
    SLAVE_DATA.write(0xFF);

    // Mask master PIC except cascade (IRQ 2).
    MASTER_DATA.write(PROBE_VAL);

    // Read back from master PIC. If the port exists, it returns what we wrote.
    // If it's a memory hole (no PIC), it usually returns 0xFF or 0x00.
    MASTER_DATA.read() == PROBE_VAL
}

fn set_mask(master_mask: u8, slave_mask: u8) {
    // Start initialization
    MASTER_CMD.write(0x11);
    SLAVE_CMD.write(0x11);

    // Set offsets
    // - Map master PIC vector 0x00~0x07 to IRQ number 0x20~0x27
    MASTER_DATA.write(IRQ_OFFSET);
    // - Map slave PIC vector 0x00~0x07 to IRQ number 0x28~0x2f
    SLAVE_DATA.write(IRQ_OFFSET + 0x08);

    // Set up cascade (there is a slave PIC at IRQ2)
    MASTER_DATA.write(4);
    SLAVE_DATA.write(2);

    // Set up interrupt mode (1 is 8086/88 mode, 2 is auto EOI)
    MASTER_DATA.write(1);
    SLAVE_DATA.write(1);

    // Mask interrupts
    MASTER_DATA.write(master_mask);
    SLAVE_DATA.write(slave_mask);
}
