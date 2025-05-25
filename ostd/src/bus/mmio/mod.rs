// SPDX-License-Identifier: MPL-2.0

#![expect(dead_code)]

//! Virtio over MMIO

pub mod bus;
pub mod common_device;

use alloc::vec::Vec;
use core::ops::Range;

use log::{debug, warn};

use self::bus::MmioBus;
use crate::{
    bus::mmio::common_device::MmioCommonDevice, mm::paddr_to_vaddr, sync::SpinLock, trap::IrqLine,
};

const VIRTIO_MMIO_MAGIC: u32 = 0x74726976;

/// MMIO bus instance
pub static MMIO_BUS: SpinLock<MmioBus> = SpinLock::new(MmioBus::new());
static IRQS: SpinLock<Vec<IrqLine>> = SpinLock::new(Vec::new());

pub(crate) fn init() {
    #[cfg(target_arch = "x86_64")]
    {
        crate::arch::if_tdx_enabled!({
            // SAFETY:
            // This is safe because we are ensuring that the address range 0xFEB0_0000 to 0xFEB0_4000 is valid before this operation.
            // The address range is page-aligned and falls within the MMIO range, which is a requirement for the `unprotect_gpa_range` function.
            // We are also ensuring that we are only unprotecting four pages.
            // Therefore, we are not causing any undefined behavior or violating any of the requirements of the `unprotect_gpa_range` function.
            unsafe {
                crate::arch::tdx_guest::unprotect_gpa_range(0xFEB0_0000, 4).unwrap();
            }
        });
        // FIXME: The address 0xFEB0_0000 is obtained from an instance of microvm, and it may not work in other architecture.
        iter_range(0xFEB0_0000..0xFEB0_4000);
    }

    #[cfg(target_arch = "riscv64")]
    {
        // An example virtio_block device taking 512 bytes at 0x1e000, interrupt 42.
        // ```dts
        // virtio_block@1e000 {
        //     compatible = "virtio,mmio";
        //     reg = <0x1e000 0x200>;
        //     interrupts = <42>;
        // }
        // ```
        iter_device_tree();
    }
}

#[cfg(target_arch = "x86_64")]
fn iter_range(range: Range<usize>) {
    debug!("[Virtio]: Iter MMIO range:{:x?}", range);
    let mut current = range.end;
    let mut lock = MMIO_BUS.lock();
    let io_apics = crate::arch::kernel::IO_APIC.get().unwrap();
    let is_ioapic2 = io_apics.len() == 2;
    let mut io_apic = if is_ioapic2 {
        io_apics.get(1).unwrap().lock()
    } else {
        io_apics.first().unwrap().lock()
    };
    let mut device_count = 0;
    while current > range.start {
        current -= 0x100;
        // SAFETY: It only read the value and judge if the magic value fit 0x74726976
        let magic = unsafe { core::ptr::read_volatile(paddr_to_vaddr(current) as *const u32) };
        if magic == VIRTIO_MMIO_MAGIC {
            // SAFETY: It only read the device id
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

#[cfg(target_arch = "riscv64")]
fn iter_device_tree() {
    debug!("[Virtio]: Iter device tree");
    use crate::arch::boot::DEVICE_TREE;

    let mut lock = MMIO_BUS.lock();
    let device_tree = DEVICE_TREE.get().unwrap();

    for node in device_tree.all_nodes() {
        let Some(compats) = node.compatible() else {
            continue;
        };
        if compats.all().any(|s| s == "virtio,mmio") {
            // Get the base address and size from the reg property
            let Some(mut reg_iter) = node.reg() else {
                continue;
            };
            let Some(region) = reg_iter.next() else {
                continue;
            };
            let base_addr = region.starting_address as usize;
            let _size = region.size.unwrap_or(0x200); // Default size if not specified

            // Check if the device has the correct magic value
            // SAFETY: It only reads the value and checks if the magic value matches 0x74726976
            use core::ptr::read_volatile;
            let magic = unsafe { read_volatile(paddr_to_vaddr(base_addr) as *const u32) };
            if magic != VIRTIO_MMIO_MAGIC {
                // required by virito-mmio specification
                warn!(
                    "[Virtio]: device at {:x} with wrong MAGIC number, got {:x}, expecting {:x}",
                    base_addr, magic, VIRTIO_MMIO_MAGIC
                );
                continue;
            }
            // Read device id
            let device_id = unsafe { *(paddr_to_vaddr(base_addr + 8) as *const u32) };
            if device_id == 0 {
                warn!(
                    "[Virtio]: device at {:x} has device_id=0, skipping",
                    base_addr
                );
                continue;
            }

            // Get the interrupt information
            let Some(mut interrupts) = node.interrupts() else {
                // required by virito-mmio specification
                warn!(
                    "[Virtio]: device_id={} without interrupts property",
                    device_id
                );
                continue;
            };
            let Some(interrupt_num) = interrupts.next() else {
                // required by virito-mmio specification
                warn!(
                    "[Virtio]: device_id={} with malformed interrupts property",
                    device_id
                );
                continue;
            };
            // let handle = IrqLine::alloc().unwrap();
            let Ok(handle) = IrqLine::alloc_specific(interrupt_num as u8) else {
                // Cannot allocate this interrupt number
                warn!(
                    "[Virtio]: unable to allocate IrqLine for device={}, interrupt number={}",
                    device_id, interrupt_num
                );
                continue;
            };
            let device = MmioCommonDevice::new(base_addr, handle);
            lock.register_mmio_device(device);
        }
    }
}
