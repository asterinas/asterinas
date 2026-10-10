// SPDX-License-Identifier: MPL-2.0

//! MSI-X capability support.

use alloc::{sync::Arc, vec::Vec};
use core::sync::atomic::{AtomicBool, Ordering};

use ostd::{Error, Result, io::IoMem, irq::IrqLine, mm::VmIoOnce};

use crate::{
    PciDeviceLocation,
    arch::{MSIX_DEFAULT_MSG_ADDR, construct_remappable_msix_address},
    cfg_space::{BarAccess, Command, PciCommonCfgOffset},
    common_device::{BarManager, PciCommonDevice},
};

// Offsets within the MSI-X capability.
const MSIX_MSG_CTRL_OFFSET: u16 = 0x02;
const MSIX_TABLE_INFO_OFFSET: u16 = 0x04;
const MSIX_PBA_INFO_OFFSET: u16 = 0x08;

// Bit masks for Message Control and the table/PBA descriptors.
const MSIX_ENABLE: u16 = 1 << 15;
const MSIX_TABLE_SIZE_MASK: u16 = 0x07ff;
const MSIX_BAR_INDEX_MASK: u32 = 0x7;

// MSI-X table entry layout, in bytes.
const MSIX_ENTRY_SIZE: usize = 16;
const MSIX_ENTRY_ADDR_OFFSET: usize = 0x00;
const MSIX_ENTRY_UPPER_ADDR_OFFSET: usize = 0x04;
const MSIX_ENTRY_DATA_OFFSET: usize = 0x08;
const MSIX_ENTRY_CTRL_OFFSET: usize = 0x0c;
const MSIX_VECTOR_MASK: u32 = 1;

/// Raw information about MSI-X capability.
#[derive(Debug)]
pub(super) struct RawCapabilityMsix {
    cap_ptr: u16,
    msg_ctrl: u16,
    table_info: u32,
    pba_info: u32,
    is_acquired: Arc<AtomicBool>,
}

impl RawCapabilityMsix {
    pub(super) fn parse(dev: &PciCommonDevice, cap_ptr: u16) -> Self {
        let msg_ctrl = dev.location().read16(cap_ptr + MSIX_MSG_CTRL_OFFSET);
        let table_info = dev.location().read32(cap_ptr + MSIX_TABLE_INFO_OFFSET);
        let pba_info = dev.location().read32(cap_ptr + MSIX_PBA_INFO_OFFSET);

        Self {
            cap_ptr,
            msg_ctrl,
            table_info,
            pba_info,
            is_acquired: Arc::new(AtomicBool::new(false)),
        }
    }
}

/// MSI-X capability.
///
/// Owns exclusive access to the MSI-X table and its IRQ lines.
/// Dropping it masks all vectors and disables MSI-X before releasing the IRQ lines.
#[derive(Debug)]
pub struct CapabilityMsixData {
    loc: PciDeviceLocation,
    ptr: u16,
    is_acquired: Arc<AtomicBool>,
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
        bar_manager: &BarManager,
        raw_cap: &RawCapabilityMsix,
    ) -> Result<Self> {
        let pba_bar = match bar_manager
            .bar((raw_cap.pba_info & MSIX_BAR_INDEX_MASK) as u8)
            .ok_or(Error::InvalidArgs)?
            .acquire()?
        {
            BarAccess::Memory(io_mem) => io_mem,
            BarAccess::Io => return Err(Error::InvalidArgs),
        };
        let pba_offset = (raw_cap.pba_info & !MSIX_BAR_INDEX_MASK) as usize;

        let table_bar = match bar_manager
            .bar((raw_cap.table_info & MSIX_BAR_INDEX_MASK) as u8)
            .ok_or(Error::InvalidArgs)?
            .acquire()?
        {
            BarAccess::Memory(io_mem) => io_mem,
            BarAccess::Io => return Err(Error::InvalidArgs),
        };
        let table_offset = (raw_cap.table_info & !MSIX_BAR_INDEX_MASK) as usize;

        let table_size = (raw_cap.msg_ctrl & MSIX_TABLE_SIZE_MASK) + 1;

        if raw_cap
            .is_acquired
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            return Err(Error::AccessDenied);
        }

        // Construct the owner before programming the table so `Drop` runs if initialization unwinds.
        let msix = Self {
            loc: *loc,
            ptr: raw_cap.cap_ptr,
            is_acquired: raw_cap.is_acquired.clone(),
            table_size,
            table_bar,
            pending_table_bar: pba_bar,
            table_offset,
            pending_table_offset: pba_offset,
            irqs: (0..table_size).map(|_| None).collect(),
        };

        // Set the message address and disable all MSI-X vectors.
        let message_address = MSIX_DEFAULT_MSG_ADDR;
        let message_upper_address = 0u32;
        for i in 0..table_size {
            let entry_offset = table_offset + usize::from(i) * MSIX_ENTRY_SIZE;
            msix.table_bar
                .write_once(entry_offset + MSIX_ENTRY_ADDR_OFFSET, &message_address)
                .unwrap();
            msix.table_bar
                .write_once(
                    entry_offset + MSIX_ENTRY_UPPER_ADDR_OFFSET,
                    &message_upper_address,
                )
                .unwrap();
            msix.table_bar
                .write_once(entry_offset + MSIX_ENTRY_CTRL_OFFSET, &MSIX_VECTOR_MASK)
                .unwrap();
        }
        // Flush table writes before enabling MSI-X.
        msix.table_bar
            .read_once::<u32>(table_offset + MSIX_ENTRY_CTRL_OFFSET)
            .unwrap();

        // Enable MSI-X.
        loc.write16(
            raw_cap.cap_ptr + MSIX_MSG_CTRL_OFFSET,
            loc.read16(raw_cap.cap_ptr + MSIX_MSG_CTRL_OFFSET) | MSIX_ENABLE,
        );
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

        let entry_offset = self.table_offset + usize::from(index) * MSIX_ENTRY_SIZE;

        // If interrupt remapping is enabled, then we need to change the value of the message address.
        if let Some(remapping_index) = irq.remapping_index() {
            let address = construct_remappable_msix_address(remapping_index as u32);

            self.table_bar
                .write_once(entry_offset + MSIX_ENTRY_ADDR_OFFSET, &address)
                .unwrap();
            self.table_bar
                .write_once(entry_offset + MSIX_ENTRY_DATA_OFFSET, &0)
                .unwrap();
        } else {
            self.table_bar
                .write_once(entry_offset + MSIX_ENTRY_DATA_OFFSET, &(irq.num() as u32))
                .unwrap();
        }

        let _old_irq = self.irqs[index as usize].replace(irq);
        // Enable this MSI-X vector.
        self.table_bar
            .write_once(entry_offset + MSIX_ENTRY_CTRL_OFFSET, &0_u32)
            .unwrap();
    }

    /// Returns a mutable reference to the [`IrqLine`].
    ///
    /// Users can register callbacks using the returned [`IrqLine`] reference.
    pub fn irq_mut(&mut self, index: usize) -> Option<&mut IrqLine> {
        self.irqs[index].as_mut()
    }

    /// Returns whether the MSI-X Enable bit is set.
    pub fn is_enabled(&self) -> bool {
        let msg_ctrl = self.loc.read16(self.ptr + MSIX_MSG_CTRL_OFFSET);
        msg_ctrl & MSIX_ENABLE != 0
    }
}

impl Drop for CapabilityMsixData {
    fn drop(&mut self) {
        // Mask all vectors and flush the writes before disabling MSI-X.
        for i in 0..self.table_size {
            let entry_offset = self.table_offset + usize::from(i) * MSIX_ENTRY_SIZE;
            self.table_bar
                .write_once(entry_offset + MSIX_ENTRY_CTRL_OFFSET, &MSIX_VECTOR_MASK)
                .unwrap();
        }
        self.table_bar
            .read_once::<u32>(self.table_offset + MSIX_ENTRY_CTRL_OFFSET)
            .unwrap();

        // Disable MSI-X and read back the control register before releasing IRQ lines.
        self.loc.write16(
            self.ptr + MSIX_MSG_CTRL_OFFSET,
            self.loc.read16(self.ptr + MSIX_MSG_CTRL_OFFSET) & !MSIX_ENABLE,
        );
        self.loc.read16(self.ptr + MSIX_MSG_CTRL_OFFSET);

        self.irqs.clear();

        // A new owner must not configure the table until cleanup is complete.
        self.is_acquired.store(false, Ordering::Release);
    }
}
