use alloc::vec::Vec;

use crate::util::{CSpaceAccessMethod, Location, BAR};
use kxos_frame_pod_derive::Pod;

use super::capability::msix::CapabilityMSIXData;

use kxos_frame::{offset_of, vm::Pod, IrqAllocateHandle};
use kxos_util::frame_ptr::InFramePtr;

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
        let table_size = table_info & (0b11_1111_1111);
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
        let mut cap = Self {
            table_size: table_size as u16,
            table: Vec::new(),
            pba_paddr: (pba_info / 8) as u64 + pba_bar_address,
        };
        let mut table_paddr = (table_info / 8) as u64 + table_bar_address;
        for _ in 0..table_size {
            let value: InFramePtr<MSIXTableEntry> =
                InFramePtr::new(table_paddr as usize).expect("can not get in frame ptr for msix");
            // let mut value = MSIXTableEntry::default();
            value.write_at(offset_of!(MSIXTableEntry, msg_addr), 0xFEE0_0000 as u32);
            // allocate irq number
            let handle = kxos_frame::allocate_irq().expect("not enough irq");
            value.write_at(offset_of!(MSIXTableEntry, msg_data), handle.num() as u32);
            value.write_at(offset_of!(MSIXTableEntry, vector_control), 0 as u32);
            cap.table.push(MSIXEntry {
                table_entry: value,
                irq_handle: handle,
            });
            table_paddr += 16;
        }
        // enable MSI-X
        let am = CSpaceAccessMethod::IO;
        am.write8(loc, cap_ptr, 0b1000_0000);
        cap
    }
}
