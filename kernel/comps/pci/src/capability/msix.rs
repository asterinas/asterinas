// SPDX-License-Identifier: MPL-2.0

//! MSI-X capability support.

use alloc::vec::Vec;

use ostd::{Error, Result, io::IoMem, irq::IrqLine, mm::VmIoOnce};

use crate::{
    PciDeviceLocation,
    arch::{MSIX_DEFAULT_MSG_ADDR, construct_remappable_msix_address},
    cfg_space::{BarAccess, Command, PciCommonCfgOffset},
    common_device::{BarManager, PciCommonDevice},
};

/// Raw information about MSI-X capability.
#[derive(Debug)]
pub(super) struct RawCapabilityMsix {
    cap_ptr: u16,
    msg_ctrl: u16,
    table_info: u32,
    pba_info: u32,
}

impl RawCapabilityMsix {
    pub(super) fn parse(dev: &PciCommonDevice, cap_ptr: u16) -> Self {
        let msg_ctrl = dev.location().read16(cap_ptr + 2);
        let table_info = dev.location().read32(cap_ptr + 4);
        let pba_info = dev.location().read32(cap_ptr + 8);

        Self {
            cap_ptr,
            msg_ctrl,
            table_info,
            pba_info,
        }
    }
}

/// MSI-X capability.
///
/// It will acquire the access to the BAR space it uses.
#[derive(Debug)]
pub struct CapabilityMsixData {
    loc: PciDeviceLocation,
    ptr: u16,
    table_size: u16,
    /// MSI-X table entry content:
    /// | Vector Control: u32 | Msg Data: u32 | Msg Upper Addr: u32 | Msg Addr: u32 |
    table_bar: IoMem,
    /// Pending bits table.
    #[expect(dead_code)]
    pending_table_bar: IoMem,
    table_offset: usize,
    #[expect(dead_code)]
    pending_table_offset: usize,
    irqs: Vec<Option<IrqLine>>,
}

impl CapabilityMsixData {
    pub(super) fn new(
        loc: &PciDeviceLocation,
        bar_manager: &mut BarManager,
        raw_cap: &RawCapabilityMsix,
    ) -> Result<Self> {
        let pba_bar = match bar_manager
            .bar_mut((raw_cap.pba_info & 0b111) as u8)
            .ok_or(Error::InvalidArgs)?
            .acquire()?
        {
            BarAccess::Memory(io_mem) => io_mem,
            BarAccess::Io => return Err(Error::InvalidArgs),
        };
        let pba_offset = (raw_cap.pba_info & !(0b111u32)) as usize;

        let table_bar = match bar_manager
            .bar_mut((raw_cap.table_info & 0b111) as u8)
            .ok_or(Error::InvalidArgs)?
            .acquire()?
        {
            BarAccess::Memory(io_mem) => io_mem,
            BarAccess::Io => return Err(Error::InvalidArgs),
        };
        let table_offset = (raw_cap.table_info & !(0b111u32)) as usize;

        let table_size = (raw_cap.msg_ctrl & 0b11_1111_1111) + 1;

        // Set the message address and disable all MSI-X vectors.
        let message_address = MSIX_DEFAULT_MSG_ADDR;
        let message_upper_address = 0u32;
        for i in 0..table_size {
            table_bar
                .write_once((16 * i) as usize + table_offset, &message_address)
                .unwrap();
            table_bar
                .write_once((16 * i + 4) as usize + table_offset, &message_upper_address)
                .unwrap();
            table_bar
                .write_once((16 * i + 12) as usize + table_offset, &1_u32)
                .unwrap();
        }

        // Enable MSI-X (bit 15: MSI-X Enable).
        loc.write16(
            raw_cap.cap_ptr + 2,
            loc.read16(raw_cap.cap_ptr + 2) | 0x8000,
        );
        // Disable INTx. Enable bus master.
        loc.write16(
            PciCommonCfgOffset::Command as u16,
            loc.read16(PciCommonCfgOffset::Command as u16)
                | (Command::INTERRUPT_DISABLE | Command::BUS_MASTER).bits(),
        );

        let mut irqs = Vec::with_capacity(table_size as usize);
        for _ in 0..table_size {
            irqs.push(None);
        }

        Ok(Self {
            loc: *loc,
            ptr: raw_cap.cap_ptr,
            table_size,
            table_bar,
            pending_table_bar: pba_bar,
            irqs,
            table_offset,
            pending_table_offset: pba_offset,
        })
    }

    /// Returns the size of the MSI-X Table.
    pub fn table_size(&self) -> u16 {
        self.table_size
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
                .write_once((16 * index) as usize + self.table_offset, &address)
                .unwrap();
            self.table_bar
                .write_once((16 * index + 8) as usize + self.table_offset, &0)
                .unwrap();
        } else {
            self.table_bar
                .write_once(
                    (16 * index + 8) as usize + self.table_offset,
                    &(irq.num() as u32),
                )
                .unwrap();
        }

        let _old_irq = self.irqs[index as usize].replace(irq);
        // Enable this MSI-X vector.
        self.table_bar
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
