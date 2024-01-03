// SPDX-License-Identifier: MPL-2.0

//! Virtio over MMIO

pub mod bus;
pub mod device;

use core::ops::Range;

use alloc::vec::Vec;
use log::debug;

use crate::{
    arch::kernel::IO_APIC, bus::mmio::device::MmioCommonDevice, sync::SpinLock, trap::IrqLine,
    vm::paddr_to_vaddr,
};

use self::bus::MmioBus;

const VIRTIO_MMIO_MAGIC: u32 = 0x74726976;

pub static MMIO_BUS: SpinLock<MmioBus> = SpinLock::new(MmioBus::new());
static IRQS: SpinLock<Vec<IrqLine>> = SpinLock::new(Vec::new());

pub fn init() {
    // FIXME: The address 0xFEB0_0000 is obtained from an instance of microvm, and it may not work in other architecture.
    iter_range(0xFEB0_0000..0xFEB0_4000);
}

fn iter_range(range: Range<usize>) {
    debug!("[Virtio]: Iter MMIO range:{:x?}", range);
    let mut current = range.end;
    let mut lock = MMIO_BUS.lock();
    let io_apics = IO_APIC.get().unwrap();
    let is_ioapic2 = io_apics.len() == 2;
    let mut io_apic = if is_ioapic2 {
        io_apics.get(1).unwrap().lock()
    } else {
        io_apics.first().unwrap().lock()
    };
    let mut device_count = 0;
    while current > range.start {
        current -= 0x100;
        // Safety: It only read the value and judge if the magic value fit 0x74726976
        let value = unsafe { *(paddr_to_vaddr(current) as *const u32) };
        if value == VIRTIO_MMIO_MAGIC {
            // Safety: It only read the device id
            let device_id = unsafe { *(paddr_to_vaddr(current + 8) as *const u32) };
            device_count += 1;
            if device_id == 0 {
                continue;
            }
            let handle = IrqLine::alloc().unwrap();
            // If has two IOApic, then start: 24 (0 in IOApic2), end 47 (23 in IOApic2)
            // If one IOApic, then start: 16, end 23
            io_apic.enable(24 - device_count, handle.clone()).unwrap();
            let device = MmioCommonDevice::new(current, handle);
            lock.register_mmio_device(device);
        }
    }
}
