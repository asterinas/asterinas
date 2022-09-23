use alloc::vec::Vec;
use pci::*;

use crate::{mm, trap::NOT_USING_IRQ_NUMBER};

use super::pci::*;

#[derive(Debug)]
#[repr(C)]
pub struct CapabilityMSIXData {
    pub cap_ptr: u16,
    pub table_size: u16,
    pub table: Vec<MSIXEntry>,
    /// pointing to the Pending Table
    pub pba_addr: u64,
}

#[derive(Debug)]
pub struct MSIXEntry {
    pub table_entry: &'static mut MSIXTableEntry,
    pub allocate_irq: u8,
}

#[derive(Debug)]
#[repr(C)]
pub struct MSIXTableEntry {
    pub msg_addr: u32,
    pub msg_upper_addr: u32,
    pub msg_data: u32,
    pub vector_control: u32,
}

impl CapabilityMSIXData {
    pub unsafe fn handle(loc: Location, cap_ptr: u16) -> Self {
        let ops = &PortOpsImpl;
        let am = CSpaceAccessMethod::IO;
        let message_control = am.read16(ops, loc, cap_ptr + 2);
        let table_info = am.read32(ops, loc, cap_ptr + 4);
        let pba_info = am.read32(ops, loc, cap_ptr + 8);
        let table_size = table_info & (0b11_1111_1111);
        let mut cap = Self {
            cap_ptr: cap_ptr,
            table_size: table_size as u16,
            table: Vec::new(),
            pba_addr: mm::phys_to_virt(
                (pba_info / 8 + am.read32(ops, loc, PCI_BAR + ((pba_info & 0b111) as u16) * 4))
                    as usize,
            ) as u64,
        };
        let mut table_addr = mm::phys_to_virt(
            (table_info / 8 + am.read32(ops, loc, PCI_BAR + ((table_info & 0b111) as u16) * 4))
                as usize,
        );
        for i in 0..table_size {
            let entry = &mut *(table_addr as *const usize as *mut MSIXTableEntry);
            entry.msg_addr = 0xFEE0_0000;
            // allocate irq number
            let irq_number = NOT_USING_IRQ_NUMBER.exclusive_access().pop().unwrap();
            entry.msg_data = irq_number as u32;
            entry.vector_control = 0;
            cap.table.push(MSIXEntry {
                table_entry: entry,
                allocate_irq: irq_number,
            });
            table_addr += 32;
        }
        // enable MSI-X
        am.write8(ops, loc, cap_ptr, 0b1000_0000);
        cap
    }
}
