// SPDX-License-Identifier: MPL-2.0

use alloc::{sync::Arc, vec::Vec};

use crate::{
    bus::pci::{
        cfg_space::{Bar, Command, MemoryBar},
        common_device::PciCommonDevice,
        device_info::PciDeviceLocation,
    },
    trap::IrqLine,
    vm::VmIo,
};

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
        let message_address = 0xFEE0_0000u32;
        let message_upper_address = 0u32;

        // Set message address 0xFEE0_0000
        for i in 0..table_size {
            // Set message address and disable this msix entry
            table_bar
                .io_mem()
                .write_val((16 * i) as usize + table_offset, &message_address)
                .unwrap();
            table_bar
                .io_mem()
                .write_val((16 * i + 4) as usize + table_offset, &message_upper_address)
                .unwrap();
            table_bar
                .io_mem()
                .write_val((16 * i + 12) as usize + table_offset, &1_u32)
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

    pub fn table_size(&self) -> u16 {
        // bit 10:0 table size
        (self.loc.read16(self.ptr + 2) & 0b11_1111_1111) + 1
    }

    pub fn set_interrupt_vector(&mut self, handle: IrqLine, index: u16) {
        if index >= self.table_size {
            return;
        }
        self.table_bar
            .io_mem()
            .write_val(
                (16 * index + 8) as usize + self.table_offset,
                &(handle.num() as u32),
            )
            .unwrap();
        let old_handles = core::mem::replace(&mut self.irqs[index as usize], Some(handle));
        // Enable this msix vector
        self.table_bar
            .io_mem()
            .write_val((16 * index + 12) as usize + self.table_offset, &0_u32)
            .unwrap();
    }

    pub fn irq_mut(&mut self, index: usize) -> Option<&mut IrqLine> {
        self.irqs[index].as_mut()
    }
}

fn set_bit(origin_value: u16, offset: usize, set: bool) -> u16 {
    (origin_value & (!(1 << offset))) | ((set as u16) << offset)
}
