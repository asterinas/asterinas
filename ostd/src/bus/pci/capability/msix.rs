// SPDX-License-Identifier: MPL-2.0

//! MSI-X capability support.

#![allow(dead_code)]
#![allow(unused_variables)]

use alloc::{sync::Arc, vec::Vec};

use cfg_if::cfg_if;

use crate::{
    arch::iommu::has_interrupt_remapping,
    bus::pci::{
        cfg_space::{Bar, Command, MemoryBar},
        common_device::PciCommonDevice,
        device_info::PciDeviceLocation,
    },
    mm::VmIoOnce,
    trap::IrqLine,
};

cfg_if! {
    if #[cfg(all(target_arch = "x86_64", feature = "cvm_guest"))] {
        use ::tdx_guest::tdx_is_enabled;
        use crate::arch::tdx_guest;
    }
}

/// MSI-X capability. It will set the BAR space it uses to be hidden.
#[derive(Debug)]
#[repr(C)]
pub struct CapabilityMsixData {
    loc: PciDeviceLocation,
    ptr: u16,
    table_size: u16,
    /// MSIX table entry content:
    /// | Vector Control: u32 | Msg Data: u32 | Msg Upper Addr: u32 | Msg Addr: u32 |
    table_bar: Arc<MemoryBar>,
    /// Pending bits table.
    pending_table_bar: Arc<MemoryBar>,
    table_offset: usize,
    pending_table_offset: usize,
    irqs: Vec<Option<IrqLine>>,
}

impl Clone for CapabilityMsixData {
    fn clone(&self) -> Self {
        let new_vec = self.irqs.clone().to_vec();
        Self {
            loc: self.loc,
            ptr: self.ptr,
            table_size: self.table_size,
            table_bar: self.table_bar.clone(),
            pending_table_bar: self.pending_table_bar.clone(),
            irqs: new_vec,
            table_offset: self.table_offset,
            pending_table_offset: self.pending_table_offset,
        }
    }
}

#[cfg(target_arch = "x86_64")]
const MSIX_DEFAULT_MSG_ADDR: u32 = 0xFEE0_0000;

impl CapabilityMsixData {
    pub(super) fn new(dev: &mut PciCommonDevice, cap_ptr: u16) -> Self {
        // Get Table and PBA offset, provide functions to modify them
        let table_info = dev.location().read32(cap_ptr + 4);
        let pba_info = dev.location().read32(cap_ptr + 8);

        let table_bar;
        let pba_bar;

        let bar_manager = dev.bar_manager_mut();
        bar_manager.set_invisible((pba_info & 0b111) as u8);
        bar_manager.set_invisible((table_info & 0b111) as u8);
        match bar_manager
            .bar_space_without_invisible((pba_info & 0b111) as u8)
            .expect("MSIX cfg:pba BAR is none")
        {
            Bar::Memory(memory) => {
                pba_bar = memory;
            }
            Bar::Io(_) => {
                panic!("MSIX cfg:pba BAR is IO type")
            }
        };
        match bar_manager
            .bar_space_without_invisible((table_info & 0b111) as u8)
            .expect("MSIX cfg:table BAR is none")
        {
            Bar::Memory(memory) => {
                table_bar = memory;
            }
            Bar::Io(_) => {
                panic!("MSIX cfg:table BAR is IO type")
            }
        }

        let pba_offset = (pba_info & !(0b111u32)) as usize;
        let table_offset = (table_info & !(0b111u32)) as usize;

        let table_size = (dev.location().read16(cap_ptr + 2) & 0b11_1111_1111) + 1;
        // TODO: Different architecture seems to have different, so we should set different address here.
        let message_address = MSIX_DEFAULT_MSG_ADDR;
        let message_upper_address = 0u32;

        // Set message address 0xFEE0_0000
        for i in 0..table_size {
            #[cfg(all(target_arch = "x86_64", feature = "cvm_guest"))]
            // SAFETY:
            // This is safe because we are ensuring that the physical address of the MSI-X table is valid before this operation.
            // We are also ensuring that we are only unprotecting a single page.
            // The MSI-X table will not exceed one page size, because the size of an MSI-X entry is 16 bytes, and 256 entries are required to fill a page,
            // which is just equal to the number of all the interrupt numbers on the x86 platform.
            // It is better to add a judgment here in case the device deliberately uses so many interrupt numbers.
            // In addition, due to granularity, the minimum value that can be set here is only one page.
            // Therefore, we are not causing any undefined behavior or violating any of the requirements of the `unprotect_gpa_range` function.
            if tdx_is_enabled() {
                unsafe {
                    tdx_guest::unprotect_gpa_range(table_bar.io_mem().paddr(), 1).unwrap();
                }
            }
            // Set message address and disable this msix entry
            table_bar
                .io_mem()
                .write_once((16 * i) as usize + table_offset, &message_address)
                .unwrap();
            table_bar
                .io_mem()
                .write_once((16 * i + 4) as usize + table_offset, &message_upper_address)
                .unwrap();
            table_bar
                .io_mem()
                .write_once((16 * i + 12) as usize + table_offset, &1_u32)
                .unwrap();
        }

        // enable MSI-X, bit15: MSI-X Enable
        dev.location()
            .write16(cap_ptr + 2, dev.location().read16(cap_ptr + 2) | 0x8000);
        // disable INTx, enable Bus master.
        dev.set_command(dev.command() | Command::INTERRUPT_DISABLE | Command::BUS_MASTER);

        let mut irqs = Vec::with_capacity(table_size as usize);
        for i in 0..table_size {
            irqs.push(None);
        }

        Self {
            loc: *dev.location(),
            ptr: cap_ptr,
            table_size: (dev.location().read16(cap_ptr + 2) & 0b11_1111_1111) + 1,
            table_bar,
            pending_table_bar: pba_bar,
            irqs,
            table_offset,
            pending_table_offset: pba_offset,
        }
    }

    /// MSI-X Table size
    pub fn table_size(&self) -> u16 {
        // bit 10:0 table size
        (self.loc.read16(self.ptr + 2) & 0b11_1111_1111) + 1
    }

    /// Enables an interrupt line, it will replace the old handle with the new handle.
    pub fn set_interrupt_vector(&mut self, irq: IrqLine, index: u16) {
        if index >= self.table_size {
            return;
        }

        // If interrupt remapping is enabled, then we need to change the value of the message address.
        if has_interrupt_remapping() {
            let mut handle = irq.inner_irq().bind_remapping_entry().unwrap().lock();

            // Enable irt entry
            let irt_entry_mut = handle.irt_entry_mut().unwrap();
            irt_entry_mut.enable_default(irq.num() as u32);

            // Use remappable format. The bits[4:3] should be always set to 1 according to the manual.
            let mut address = MSIX_DEFAULT_MSG_ADDR | 0b1_1000;

            // Interrupt index[14:0] is on address[19:5] and interrupt index[15] is on address[2].
            address |= (handle.index() as u32 & 0x7FFF) << 5;
            address |= (handle.index() as u32 & 0x8000) >> 13;

            self.table_bar
                .io_mem()
                .write_once((16 * index) as usize + self.table_offset, &address)
                .unwrap();
            self.table_bar
                .io_mem()
                .write_once((16 * index + 8) as usize + self.table_offset, &0)
                .unwrap();
        } else {
            self.table_bar
                .io_mem()
                .write_once(
                    (16 * index + 8) as usize + self.table_offset,
                    &(irq.num() as u32),
                )
                .unwrap();
        }

        let _old_irq = core::mem::replace(&mut self.irqs[index as usize], Some(irq));
        // Enable this msix vector
        self.table_bar
            .io_mem()
            .write_once((16 * index + 12) as usize + self.table_offset, &0_u32)
            .unwrap();
    }

    /// Gets mutable IrqLine. User can register callbacks by using this function.
    pub fn irq_mut(&mut self, index: usize) -> Option<&mut IrqLine> {
        self.irqs[index].as_mut()
    }

    /// Returns true if MSI-X Enable bit is set.
    pub fn is_enabled(&self) -> bool {
        let msg_ctrl = self.loc.read16(self.ptr + 2);
        msg_ctrl & 0x8000 != 0
    }
}

fn set_bit(origin_value: u16, offset: usize, set: bool) -> u16 {
    (origin_value & (!(1 << offset))) | ((set as u16) << offset)
}
