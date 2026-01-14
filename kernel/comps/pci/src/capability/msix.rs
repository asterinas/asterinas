// SPDX-License-Identifier: MPL-2.0

//! MSI-X capability support.

use alloc::{sync::Arc, vec::Vec};

use ostd::{irq::IrqLine, mm::VmIoOnce};

use crate::{
    PciDeviceLocation,
    arch::{MSIX_DEFAULT_MSG_ADDR, construct_remappable_msix_address},
    cfg_space::{Bar, Command, MemoryBar},
    common_device::PciCommonDevice,
};

/// MSI-X capability. It will set the BAR space it uses to be hidden.
#[derive(Debug)]
pub struct CapabilityMsixData {
    loc: PciDeviceLocation,
    ptr: u16,
    table_size: u16,
    /// MSI-X table entry content:
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

        let bar_manager = dev.bar_manager();
        match bar_manager
            .bar((pba_info & 0b111) as u8)
            .clone()
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
            .bar((table_info & 0b111) as u8)
            .clone()
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

        // Set the message address and disable all MSI-X vectors.
        let message_address = MSIX_DEFAULT_MSG_ADDR;
        let message_upper_address = 0u32;
        for i in 0..table_size {
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

        // Enable MSI-X (bit 15: MSI-X Enable).
        dev.location()
            .write16(cap_ptr + 2, dev.location().read16(cap_ptr + 2) | 0x8000);
        // Disable INTx. Enable bus master.
        dev.write_command(dev.read_command() | Command::INTERRUPT_DISABLE | Command::BUS_MASTER);

        let mut irqs = Vec::with_capacity(table_size as usize);
        for _ in 0..table_size {
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

    /// Returns the size of the MSI-X Table.
    pub fn table_size(&self) -> u16 {
        // bit 10:0 table size
        (self.loc.read16(self.ptr + 2) & 0b11_1111_1111) + 1
    }

    /// Enables an interrupt line.
    ///
    /// If the interrupt line has already been enabled, the old [`IrqLine`] will be replaced.
    pub fn set_interrupt_vector(&mut self, irq: IrqLine, index: u16) {
        if index >= self.table_size {
            return;
        }

        // If interrupt remapping is enabled, then we need to change the value of the message address.
        if let Some(remapping_index) = irq.remapping_index() {
            let address = construct_remappable_msix_address(remapping_index as u32);

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

        let _old_irq = self.irqs[index as usize].replace(irq);
        // Enable this MSI-X vector.
        self.table_bar
            .io_mem()
            .write_once((16 * index + 12) as usize + self.table_offset, &0_u32)
            .unwrap();
    }

    /// Returns a mutable reference to the [`IrqLine`].
    ///
    /// Users can register callbacks using the returned [`IrqLine`] reference.
    pub fn irq_mut(&mut self, index: usize) -> Option<&mut IrqLine> {
        self.irqs[index].as_mut()
    }

    /// Returns true if the MSI-X Enable bit is set.
    pub fn is_enabled(&self) -> bool {
        let msg_ctrl = self.loc.read16(self.ptr + 2);
        msg_ctrl & 0x8000 != 0
    }
}
