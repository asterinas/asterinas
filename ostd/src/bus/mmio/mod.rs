// SPDX-License-Identifier: MPL-2.0

#![allow(dead_code)]

//! Virtio over MMIO

pub mod bus;
pub mod common_device;

use alloc::vec::Vec;
use core::ops::Range;

use cfg_if::cfg_if;
use log::debug;

use self::bus::MmioBus;
use crate::{
    bus::mmio::common_device::MmioCommonDevice, mm::paddr_to_vaddr, sync::SpinLock, trap::IrqLine,
};

cfg_if! {
    if #[cfg(all(target_arch = "x86_64", feature = "cvm_guest"))] {
        use ::tdx_guest::tdx_is_enabled;
        use crate::arch::tdx_guest;
    }
}

const VIRTIO_MMIO_MAGIC: u32 = 0x74726976;

/// MMIO bus instance
pub static MMIO_BUS: SpinLock<MmioBus> = SpinLock::new(MmioBus::new());
static IRQS: SpinLock<Vec<IrqLine>> = SpinLock::new(Vec::new());

pub(crate) fn init() {
    #[cfg(all(target_arch = "x86_64", feature = "cvm_guest"))]
    // SAFETY:
    // This is safe because we are ensuring that the address range 0xFEB0_0000 to 0xFEB0_4000 is valid before this operation.
    // The address range is page-aligned and falls within the MMIO range, which is a requirement for the `unprotect_gpa_range` function.
    // We are also ensuring that we are only unprotecting four pages.
    // Therefore, we are not causing any undefined behavior or violating any of the requirements of the `unprotect_gpa_range` function.
    if tdx_is_enabled() {
        unsafe {
            tdx_guest::unprotect_gpa_range(0xFEB0_0000, 4).unwrap();
        }
    }
    // FIXME: The address 0xFEB0_0000 is obtained from an instance of microvm, and it may not work in other architecture.
    #[cfg(target_arch = "x86_64")]
    iter_range(0xFEB0_0000..0xFEB0_4000);
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
