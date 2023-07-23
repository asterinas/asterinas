use alloc::{sync::Arc, vec::Vec};

use crate::{
    bus::pci::{
        cfg_space::{Bar, MemoryBar},
        common_device::PciCommonDevice,
        device_info::PciDeviceLocation,
    },
    trap::IrqAllocateHandle,
    vm::VmIo,
};

/// MSI-X capability. It will set the BAR space it uses to be hidden.
#[derive(Debug, Clone)]
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
    irq_allocate_handles: Vec<Option<Arc<IrqAllocateHandle>>>,
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

        let table_size = (dev.location().read16(cap_ptr + 2) & 0b11_1111_1111) + 1;
        // TODO: Different architecture seems to have different, so we should set different address here.
        let message_address = 0xFEE0_000 as u32;
        let message_upper_address = 0 as u32;

        // Set message address 0xFEE0_0000
        for i in 0..table_size {
            // Set message address and disable this msix entry
            table_bar
                .io_mem()
                .write_val((16 * i) as usize, &message_address)
                .unwrap();
            table_bar
                .io_mem()
                .write_val((16 * i + 4) as usize, &message_upper_address)
                .unwrap();
            table_bar
                .io_mem()
                .write_val((16 * i + 12) as usize, &(1 as u32))
                .unwrap();
        }

        let mut irq_allocate_handles = Vec::with_capacity(table_size as usize);
        for i in 0..table_size {
            irq_allocate_handles.push(None);
        }

        Self {
            loc: dev.location().clone(),
            ptr: cap_ptr,
            table_size: (dev.location().read16(cap_ptr + 2) & 0b11_1111_1111) + 1,
            table_bar,
            pending_table_bar: pba_bar,
            irq_allocate_handles,
        }
    }

    pub fn table_size(&self) -> u16 {
        // bit 10:0 table size
        (self.loc.read16(self.ptr + 2) & 0b11_1111_1111) + 1
    }

    pub fn set_msix_enable(&self, enable: bool) {
        // bit15: msix enable
        let value = (enable as u16) << 15;
        // message control
        self.loc.write16(
            self.ptr + 2,
            set_bit(self.loc.read16(self.ptr + 2), 15, enable),
        )
    }

    pub fn set_interrupt_enable(&self, enable: bool) {
        // bit14: msix enable
        let value = (enable as u16) << 14;
        // message control
        self.loc.write16(
            self.ptr + 2,
            set_bit(self.loc.read16(self.ptr + 2), 14, enable),
        )
    }

    pub fn set_interrupt_vector(&mut self, vector: Arc<IrqAllocateHandle>, index: u16) {
        if index >= self.table_size {
            return;
        }
        let old_handles =
            core::mem::replace(&mut self.irq_allocate_handles[index as usize], Some(vector));
        // Enable this msix vector
        self.table_bar
            .io_mem()
            .write_val((16 * index + 12) as usize, &(0 as u32))
            .unwrap();
    }
}

fn set_bit(origin_value: u16, offset: usize, set: bool) -> u16 {
    (origin_value & (!(1 << offset))) | ((set as u16) << offset)
}
