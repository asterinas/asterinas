// SPDX-License-Identifier: MPL-2.0

//! MSI-X capability support.

use alloc::vec::Vec;
use core::mem::offset_of;

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
    msg_ctrl: MsgCtrl,
    table_info: BarInfo,
    pba_info: BarInfo,
}

impl RawCapabilityMsix {
    pub(super) fn parse(dev: &PciCommonDevice, cap_ptr: u16) -> Self {
        let loc = dev.location();
        let msg_ctrl = MsgCtrl(loc.read16(cap_ptr + offset_of!(Cap, msg_ctrl) as u16));
        let table_info = BarInfo(loc.read32(cap_ptr + offset_of!(Cap, table_info) as u16));
        let pba_info = BarInfo(loc.read32(cap_ptr + offset_of!(Cap, pba_info) as u16));

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
    /// BAR containing the MSI-X table of [`Entry`] records.
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
        bar_manager: &BarManager,
        raw_cap: &RawCapabilityMsix,
    ) -> Result<Self> {
        let pba_bar = match bar_manager
            .bar(raw_cap.pba_info.bar_index())
            .ok_or(Error::InvalidArgs)?
            .acquire()?
        {
            BarAccess::Memory(io_mem) => io_mem,
            BarAccess::Io => return Err(Error::InvalidArgs),
        };
        let pba_offset = raw_cap.pba_info.offset();

        let table_bar = match bar_manager
            .bar(raw_cap.table_info.bar_index())
            .ok_or(Error::InvalidArgs)?
            .acquire()?
        {
            BarAccess::Memory(io_mem) => io_mem,
            BarAccess::Io => return Err(Error::InvalidArgs),
        };
        let table_offset = raw_cap.table_info.offset();

        let table_size = raw_cap.msg_ctrl.table_size();

        let mut irqs = Vec::with_capacity(table_size as usize);
        for _ in 0..table_size {
            irqs.push(None);
        }

        let msix = Self {
            loc: *loc,
            ptr: raw_cap.cap_ptr,
            table_size,
            table_bar,
            pending_table_bar: pba_bar,
            table_offset,
            pending_table_offset: pba_offset,
            irqs,
        };

        // Set the message address and disable all MSI-X vectors.
        let message_address = MSIX_DEFAULT_MSG_ADDR;
        let message_upper_address = 0u32;
        for i in 0..table_size {
            let entry_offset = msix.entry_offset(i);
            msix.table_bar
                .write_once(entry_offset + offset_of!(Entry, addr), &message_address)
                .unwrap();
            msix.table_bar
                .write_once(
                    entry_offset + offset_of!(Entry, upper_addr),
                    &message_upper_address,
                )
                .unwrap();
            msix.table_bar
                .write_once(entry_offset + offset_of!(Entry, ctrl), &Entry::CTRL_MASKED)
                .unwrap();
        }

        msix.set_enabled(true);
        // Disable INTx. Enable bus master.
        loc.write16(
            PciCommonCfgOffset::Command as u16,
            loc.read16(PciCommonCfgOffset::Command as u16)
                | (Command::INTERRUPT_DISABLE | Command::BUS_MASTER).bits(),
        );

        Ok(msix)
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

        let entry_offset = self.entry_offset(index);

        // If interrupt remapping is enabled, then we need to change the value of the message address.
        if let Some(remapping_index) = irq.remapping_index() {
            let address = construct_remappable_msix_address(remapping_index as u32);

            self.table_bar
                .write_once(entry_offset + offset_of!(Entry, addr), &address)
                .unwrap();
            self.table_bar
                .write_once(entry_offset + offset_of!(Entry, data), &0)
                .unwrap();
        } else {
            self.table_bar
                .write_once(entry_offset + offset_of!(Entry, data), &(irq.num() as u32))
                .unwrap();
        }

        let _old_irq = self.irqs[index as usize].replace(irq);
        // Enable this MSI-X vector.
        self.table_bar
            .write_once(
                entry_offset + offset_of!(Entry, ctrl),
                &Entry::CTRL_UNMASKED,
            )
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
        let msg_ctrl = MsgCtrl(self.loc.read16(self.ptr + offset_of!(Cap, msg_ctrl) as u16));
        msg_ctrl.is_enabled()
    }

    fn set_enabled(&self, is_enabled: bool) {
        let loc = self.loc;
        let offset = self.ptr + offset_of!(Cap, msg_ctrl) as u16;

        let msg_ctrl = loc.read16(offset);
        let msg_ctrl = if is_enabled {
            msg_ctrl | MsgCtrl::ENABLE
        } else {
            msg_ctrl & !MsgCtrl::ENABLE
        };
        loc.write16(offset, msg_ctrl);
    }

    fn entry_offset(&self, index: u16) -> usize {
        self.table_offset + usize::from(index) * size_of::<Entry>()
    }
}

/// MSI-X capability layout in PCI configuration space.
#[repr(C)]
struct Cap {
    cap_id: u8,
    next_ptr: u8,
    msg_ctrl: u16,
    table_info: u32,
    pba_info: u32,
}

/// MSI-X Message Control register.
#[derive(Clone, Copy, Debug)]
struct MsgCtrl(u16);

impl MsgCtrl {
    const ENABLE: u16 = 1 << 15;
    const TABLE_SIZE_MASK: u16 = 0x07ff;

    fn table_size(self) -> u16 {
        // The field encodes the number of entries minus one.
        (self.0 & Self::TABLE_SIZE_MASK) + 1
    }

    fn is_enabled(self) -> bool {
        self.0 & Self::ENABLE != 0
    }
}

/// BAR index and byte offset of the MSI-X table or pending bit array.
#[derive(Clone, Copy, Debug)]
struct BarInfo(u32);

impl BarInfo {
    const BAR_INDEX_MASK: u32 = 0x7;

    fn bar_index(self) -> u8 {
        (self.0 & Self::BAR_INDEX_MASK) as u8
    }

    fn offset(self) -> usize {
        (self.0 & !Self::BAR_INDEX_MASK) as usize
    }
}

/// MSI-X table entry layout.
#[repr(C)]
struct Entry {
    addr: u32,
    upper_addr: u32,
    data: u32,
    ctrl: u32,
}

impl Entry {
    // Whether the MSI-X vector is masked or unmasked.
    const CTRL_MASKED: u32 = 1;
    const CTRL_UNMASKED: u32 = 0;
}
