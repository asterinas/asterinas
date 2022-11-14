//! The pci of kxos
#![no_std]
#![forbid(unsafe_code)]
#![allow(dead_code)]
pub mod capability;
pub mod msix;
pub mod util;
extern crate alloc;
use kxos_frame::info;
extern crate kxos_frame_pod_derive;

use alloc::{sync::Arc, vec::Vec};
use lazy_static::lazy_static;
use spin::mutex::Mutex;
use util::CSpaceAccessMethod;

pub use crate::util::PCIDevice;

pub const PCI_COMMAND: u16 = 0x04;
pub const PCI_BAR: u16 = 0x10;
pub const PCI_CAP_PTR: u16 = 0x34;
pub const PCI_INTERRUPT_LINE: u16 = 0x3c;
pub const PCI_INTERRUPT_PIN: u16 = 0x3d;

pub const PCI_MSIX_CTRL_CAP: u16 = 0x00;
pub const PCI_MSIX_TABLE: u16 = 0x04;
pub const PCI_MSIX_PBA: u16 = 0x08;

pub const PCI_CAP_ID_MSI: u8 = 0x05;

lazy_static! {
    static ref PCI_DEVICES: Mutex<Vec<Arc<PCIDevice>>> = Mutex::new(Vec::new());
}
pub fn init() {
    if device_amount() > 0 {
        panic!("initialize pci device twice time")
    }
    let mut devices = PCI_DEVICES.lock();
    for dev in util::scan_bus(CSpaceAccessMethod::IO) {
        info!(
            "pci: {:02x}:{:02x}.{} {:#x} {:#x} ({} {}) irq: {}:{:?}",
            dev.loc.bus,
            dev.loc.device,
            dev.loc.function,
            dev.id.vendor_id,
            dev.id.device_id,
            dev.id.class,
            dev.id.subclass,
            dev.pic_interrupt_line,
            dev.interrupt_pin
        );
        devices.push(Arc::new(dev));
    }
}

pub fn get_pci_devices(index: usize) -> Option<Arc<PCIDevice>> {
    PCI_DEVICES.lock().get(index).cloned()
}

pub fn device_amount() -> usize {
    PCI_DEVICES.lock().len()
}
