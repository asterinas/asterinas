//! The pci of jinux
#![no_std]
#![forbid(unsafe_code)]
#![allow(dead_code)]
pub mod capability;
pub mod msix;
pub mod util;
extern crate alloc;

use component::init_component;
use component::ComponentInitError;
extern crate pod_derive;

use alloc::{sync::Arc, vec::Vec};
use spin::{mutex::Mutex, Once};
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

pub static PCI_COMPONENT: Once<PCIComponent> = Once::new();

#[init_component]
fn pci_component_init() -> Result<(), ComponentInitError> {
    let a = PCIComponent::init()?;
    PCI_COMPONENT.call_once(|| a);
    Ok(())
}
pub struct PCIComponent {
    pci_device: Mutex<Vec<Arc<PCIDevice>>>,
}

impl PCIComponent {
    pub fn init() -> Result<Self, ComponentInitError> {
        let mut devices = Vec::new();
        for dev in util::scan_bus(CSpaceAccessMethod::IO) {
            log::info!(
                "pci: {:02x}:{:02x}.{} {:#x} {:#x} ({} {}) command: {:?} status: {:?} irq: {}:{:?}",
                dev.loc.bus,
                dev.loc.device,
                dev.loc.function,
                dev.id.vendor_id,
                dev.id.device_id,
                dev.id.class,
                dev.id.subclass,
                dev.command,
                dev.status,
                dev.pic_interrupt_line,
                dev.interrupt_pin
            );
            devices.push(Arc::new(dev));
        }
        Ok(Self {
            pci_device: Mutex::new(devices),
        })
    }

    pub const fn name() -> &'static str {
        "PCI"
    }
    // 0~65535
    pub const fn priority() -> u16 {
        0
    }
}

impl PCIComponent {
    pub fn get_pci_devices(self: &Self, index: usize) -> Option<Arc<PCIDevice>> {
        self.pci_device.lock().get(index).cloned()
    }

    pub fn device_amount(self: &Self) -> usize {
        self.pci_device.lock().len()
    }
}
