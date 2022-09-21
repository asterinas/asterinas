use crate::{drivers::virtio, trap::NOT_USING_IRQ_NUMBER, *};
use pci::*;

pub(crate) const PCI_COMMAND: u16 = 0x04;
pub(crate) const PCI_BAR: u16 = 0x10;
pub(crate) const PCI_CAP_PTR: u16 = 0x34;
pub(crate) const PCI_INTERRUPT_LINE: u16 = 0x3c;
pub(crate) const PCI_INTERRUPT_PIN: u16 = 0x3d;

pub(crate) const PCI_MSIX_CTRL_CAP: u16 = 0x00;
pub(crate) const PCI_MSIX_TABLE: u16 = 0x04;
pub(crate) const PCI_MSIX_PBA: u16 = 0x08;

pub(crate) const PCI_CAP_ID_MSI: u8 = 0x05;

pub(crate) struct PortOpsImpl;

impl PortOps for PortOpsImpl {
    unsafe fn read8(&self, port: u16) -> u8 {
        x86_64_util::in8(port)
    }
    unsafe fn read16(&self, port: u16) -> u16 {
        x86_64_util::in16(port)
    }
    unsafe fn read32(&self, port: u16) -> u32 {
        x86_64_util::in32(port)
    }
    unsafe fn write8(&self, port: u16, val: u8) {
        x86_64_util::out8(port, val);
    }
    unsafe fn write16(&self, port: u16, val: u16) {
        x86_64_util::out16(port, val);
    }
    unsafe fn write32(&self, port: u16, val: u32) {
        x86_64_util::out32(port, val);
    }
}

pub fn init() {
    for dev in unsafe { scan_bus(&PortOpsImpl, CSpaceAccessMethod::IO) } {
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
        if dev.id.vendor_id == 0x1af4
            && (dev.id.device_id == 0x1001 || dev.id.device_id == 0x1042)
            && dev.id.class == 0x01
        {
            // virtio block device mass storage
            info!("found virtio pci block device");
            virtio::block::init(dev);
        }
    }
}
