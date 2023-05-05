use alloc::vec::Vec;
use log::debug;
use pod::Pod;

use crate::util::{CSpaceAccessMethod, Location, BAR};

use super::capability::msix::CapabilityMSIXData;

use jinux_frame::{offset_of, trap::IrqAllocateHandle};
use jinux_util::frame_ptr::InFramePtr;

#[derive(Debug, Default)]
pub struct MSIX {
    pub table_size: u16,
    pub table: Vec<MSIXEntry>,
    /// pointing to the Pending Table
    pub pba_paddr: u64,
}

#[derive(Debug)]
pub struct MSIXEntry {
    pub table_entry: InFramePtr<MSIXTableEntry>,
    pub irq_handle: IrqAllocateHandle,
}

#[derive(Debug, Default, Copy, Clone, PartialEq, Eq, Pod)]
#[repr(C)]
pub struct MSIXTableEntry {
    pub msg_addr: u32,
    pub msg_upper_addr: u32,
    pub msg_data: u32,
    pub vector_control: u32,
}

impl MSIX {
    /// create a MSIX instance, it allocate the irq number automatically.
    pub fn new(
        cap: &CapabilityMSIXData,
        bars: [Option<BAR>; 6],
        loc: Location,
        cap_ptr: u16,
    ) -> Self {
        let table_info = cap.table_info;
        let pba_info = cap.pba_info;
        let table_size = (table_info & (0b11_1111_1111)) + 1;
        let table_bar_address;
        let pba_bar_address;
        match bars[(pba_info & 0b111) as usize].expect("MSIX cfg:table bar is none") {
            BAR::Memory(address, _, _, _) => {
                pba_bar_address = address;
            }
            BAR::IO(_, _) => {
                panic!("MSIX cfg:table bar is IO type")
            }
        }
        match bars[(table_info & 0b111) as usize].expect("MSIX cfg:table bar is none") {
            BAR::Memory(address, _, _, _) => {
                table_bar_address = address;
            }
            BAR::IO(_, _) => {
                panic!("MSIX cfg:table bar is IO type")
            }
        }
        // let pba_base_address = (pba_info >> 3 ) as u64 + pba_bar_address;
        // let table_base_address = (table_info >>3 ) as u64 + table_bar_address;
        let pba_base_address = (pba_info & (!(0b111 as u32))) as u64 + pba_bar_address;
        let table_base_address = (table_info & (!(0b111 as u32))) as u64 + table_bar_address;
        debug!("MSIX table size:{}, pba_info:{:x}, table_info:{:x}, pba_address:{:x}, table_address:{:x}",
            table_size,pba_info,table_info,pba_base_address,table_base_address);
        let mut cap = Self {
            table_size: table_size as u16,
            table: Vec::new(),
            pba_paddr: pba_base_address,
        };
        // enable MSI-X disable INTx
        let am = CSpaceAccessMethod::IO;
        debug!("command before:{:x}", am.read16(loc, crate::PCI_COMMAND));
        am.write16(
            loc,
            crate::PCI_COMMAND,
            am.read16(loc, crate::PCI_COMMAND) | 0x40f,
        );
        debug!("command after:{:x}", am.read16(loc, crate::PCI_COMMAND));
        let message_control = am.read16(loc, cap_ptr + 2) | 0x8000;
        am.write16(loc, cap_ptr + 2, message_control);
        let mut table_iter: InFramePtr<MSIXTableEntry> =
            InFramePtr::new(table_base_address as usize)
                .expect("can not get in frame ptr for msix");
        for _ in 0..table_size {
            // local APIC address: 0xFEE0_0000
            table_iter.write_at(offset_of!(MSIXTableEntry, msg_addr), 0xFEE0_0000 as u32);
            table_iter.write_at(offset_of!(MSIXTableEntry, msg_upper_addr), 0 as u32);
            // allocate irq number
            let handle = jinux_frame::trap::allocate_irq().expect("not enough irq");
            table_iter.write_at(offset_of!(MSIXTableEntry, msg_data), handle.num() as u32);
            table_iter.write_at(offset_of!(MSIXTableEntry, vector_control), 0 as u32);
            cap.table.push(MSIXEntry {
                table_entry: table_iter.clone(),
                irq_handle: handle,
            });
            table_iter = table_iter.add(1);
        }
        cap
    }
}
