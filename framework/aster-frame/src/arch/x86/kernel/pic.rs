// SPDX-License-Identifier: MPL-2.0

#![allow(dead_code)]

use core::sync::atomic::{AtomicBool, AtomicU8, Ordering::Relaxed};

use log::info;

use crate::{
    arch::x86::device::io_port::{IoPort, WriteOnlyAccess},
    trap::IrqLine,
};

static MASTER_CMD: IoPort<u8, WriteOnlyAccess> = unsafe { IoPort::new(0x20) };
static MASTER_DATA: IoPort<u8, WriteOnlyAccess> = unsafe { IoPort::new(0x21) };
static SLAVE_CMD: IoPort<u8, WriteOnlyAccess> = unsafe { IoPort::new(0xA0) };
static SLAVE_DATA: IoPort<u8, WriteOnlyAccess> = unsafe { IoPort::new(0xA1) };

const IRQ_OFFSET: u8 = 0x20;

static MASK_MASTER: AtomicU8 = AtomicU8::new(0x00);
static MASK_SLAVE: AtomicU8 = AtomicU8::new(0x00);
static CHANGE_LOCK: AtomicBool = AtomicBool::new(false);

/// Initializes the PIC device
pub fn init() {
    if CHANGE_LOCK.load(Relaxed) {
        return;
    }
    let master_mask = !(MASK_MASTER.load(Relaxed));
    let slave_mask = !(MASK_SLAVE.load(Relaxed));
    info!(
        "PIC init, master mask:{:x} slave_mask:{:x}",
        master_mask, slave_mask
    );
    set_mask(master_mask, slave_mask);
}

/// Allocates irq, for example, if timer need IRQ0, it will return IrqAllocateHandle with irq num: IRQ_OFFSET+0
pub fn allocate_irq(index: u8) -> Option<IrqLine> {
    if index >= 16 {
        return None;
    }
    if let Ok(irq) = IrqLine::alloc_specific(IRQ_OFFSET + index) {
        if index >= 8 {
            MASK_SLAVE.fetch_or(1 << (index - 8), Relaxed);
        } else {
            MASK_MASTER.fetch_or(1 << (index), Relaxed);
        }
        Some(irq)
    } else {
        None
    }
}

/// Enables the PIC device, this function will permanent enable all the interrupts
#[inline]
pub fn enable() {
    CHANGE_LOCK.store(true, Relaxed);
    set_mask(0, 0);
}

/// Disables the PIC device, this function will permanent disable all the interrupts
/// the interrupts mask may not exists after calling init function
#[inline]
pub fn disable() {
    CHANGE_LOCK.store(true, Relaxed);
    set_mask(0xFF, 0xFF);
}

/// Enables the PIC device, this function will allow all the interrupts
/// the interrupts mask may not exists after calling init function
#[inline]
pub fn enable_temp() {
    set_mask(0, 0);
}

/// Disables the PIC device, this function will disable all the interrupts
/// the interrupts mask may not exists after calling init function
#[inline]
pub fn disable_temp() {
    set_mask(0xFF, 0xFF);
}

#[inline(always)]
pub fn set_mask(master_mask: u8, slave_mask: u8) {
    // Start initialization
    MASTER_CMD.write(0x11);
    SLAVE_CMD.write(0x11);

    // Set offsets
    // map master PIC vector 0x00~0x07 to 0x20~0x27 IRQ number
    MASTER_DATA.write(IRQ_OFFSET);
    // map slave PIC vector 0x00~0x07 to 0x28~0x2f IRQ number
    SLAVE_DATA.write(IRQ_OFFSET + 0x08);

    // Set up cascade, there is slave at IRQ2
    MASTER_DATA.write(4);
    SLAVE_DATA.write(2);

    // Set up interrupt mode (1 is 8086/88 mode, 2 is auto EOI)
    MASTER_DATA.write(1);
    SLAVE_DATA.write(1);

    // mask interrupts
    MASTER_DATA.write(master_mask);
    SLAVE_DATA.write(slave_mask);
}

#[inline(always)]
pub(crate) fn ack() {
    MASTER_CMD.write(0x20);
}
