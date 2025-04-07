// SPDX-License-Identifier: MPL-2.0

#![expect(dead_code)]

use alloc::collections::BTreeMap;
use core::mem::size_of;

use log::trace;
use ostd_pod::Pod;

use super::second_stage::IommuPtConfig;
use crate::{
    bus::pci::PciDeviceLocation,
    mm::{
        dma::Daddr,
        page_prop::{CachePolicy, PageProperty, PrivilegedPageFlags as PrivFlags},
        page_table::PageTableError,
        Frame, FrameAllocOptions, Paddr, PageFlags, PageTable, VmIo, PAGE_SIZE,
    },
    task::disable_preempt,
};

/// Bit 0 is `Present` bit, indicating whether this entry is present.
/// Bit 63:12 is the context-table pointer pointing to this bus's context-table.
#[derive(Pod, Clone, Copy)]
#[repr(C)]
pub struct RootEntry(u128);

impl RootEntry {
    pub const fn is_present(&self) -> bool {
        // Bit 0
        (self.0 & 0b1) != 0
    }

    pub const fn addr(&self) -> u64 {
        (self.0 & 0xFFFF_FFFF_FFFF_F000) as u64
    }
}

pub struct RootTable {
    /// Total 256 bus, each entry is 128 bits.
    root_frame: Frame<()>,
    // TODO: Use radix tree instead.
    context_tables: BTreeMap<Paddr, ContextTable>,
}

#[derive(Debug)]
pub enum ContextTableError {
    InvalidDeviceId,
    /// Error when modifying the page table
    ModificationError(PageTableError),
}

impl RootTable {
    pub fn root_paddr(&self) -> Paddr {
        self.root_frame.start_paddr()
    }

    pub(super) fn new() -> Self {
        Self {
            root_frame: FrameAllocOptions::new().alloc_frame().unwrap(),
            context_tables: BTreeMap::new(),
        }
    }

    /// Mapping device address to physical address.
    ///
    /// # Safety
    ///
    /// User must ensure the given paddr is a valid one.
    pub(super) unsafe fn map(
        &mut self,
        device: PciDeviceLocation,
        daddr: Daddr,
        paddr: Paddr,
    ) -> Result<(), ContextTableError> {
        if device.device >= 32 || device.function >= 8 {
            return Err(ContextTableError::InvalidDeviceId);
        }

        let context_table = self.get_or_create_context_table(device);
        // SAFETY: The safety is upheld by the caller.
        unsafe { context_table.map(device, daddr, paddr)? };

        Ok(())
    }

    pub(super) fn unmap(
        &mut self,
        device: PciDeviceLocation,
        daddr: Daddr,
    ) -> Result<(), ContextTableError> {
        if device.device >= 32 || device.function >= 8 {
            return Err(ContextTableError::InvalidDeviceId);
        }

        let context_table = self.get_or_create_context_table(device);
        context_table.unmap(device, daddr)?;

        Ok(())
    }

    /// Specifies the device page table instead of creating a page table if not exists.
    ///
    /// This will be useful if we want all the devices to use the same page table.
    /// The original page table will be overwritten.
    pub(super) fn specify_device_page_table(
        &mut self,
        device_id: PciDeviceLocation,
        page_table: PageTable<IommuPtConfig>,
    ) {
        let context_table = self.get_or_create_context_table(device_id);

        let bus_entry = context_table
            .entries_frame
            .read_val::<ContextEntry>(
                (device_id.device as usize * 8 + device_id.function as usize)
                    * size_of::<ContextEntry>(),
            )
            .unwrap();

        if bus_entry.is_present() {
            panic!("existing device page tables should not be overridden");
        }

        // Activate page table.
        let address = page_table.root_paddr();
        context_table.page_tables.insert(address, page_table);
        let entry = ContextEntry(address as u128 | 1 | 0x1_0000_0000_0000_0000);
        context_table
            .entries_frame
            .write_val::<ContextEntry>(
                (device_id.device as usize * 8 + device_id.function as usize)
                    * size_of::<ContextEntry>(),
                &entry,
            )
            .unwrap();
    }

    fn get_or_create_context_table(&mut self, device_id: PciDeviceLocation) -> &mut ContextTable {
        let bus_entry = self
            .root_frame
            .read_val::<RootEntry>(device_id.bus as usize * size_of::<RootEntry>())
            .unwrap();

        if !bus_entry.is_present() {
            let table = ContextTable::new();
            let address = table.paddr();
            self.context_tables.insert(address, table);
            let entry = RootEntry(address as u128 | 1);
            self.root_frame
                .write_val::<RootEntry>(device_id.bus as usize * size_of::<RootEntry>(), &entry)
                .unwrap();
            self.context_tables.get_mut(&address).unwrap()
        } else {
            self.context_tables
                .get_mut(&(bus_entry.addr() as usize))
                .unwrap()
        }
    }
}

/// Context Entry in the Context Table, used in Intel iommu.
///
/// The format of context entry:
/// ```
/// 127--88: Reserved.
/// 87---72: Domain Identifier.
/// 71---71: Reserved.
/// 70---67: Ignored.
/// 66---64: Address Width.
/// 63---12: Second Stage Page Translation Pointer.
/// 11----4: Reserved.
/// 3-----2: Translation Type.
/// 1-----1: Fault Processing Disable.
/// 0-----0: Present
/// ```
///
#[derive(Pod, Clone, Copy)]
#[repr(C)]
pub struct ContextEntry(u128);

impl ContextEntry {
    /// Identifier for the domain to which this context-entry maps. Hardware may use the domain
    /// identifier to tag its internal caches
    pub const fn domain_identifier(&self) -> u64 {
        // Bit 87-72
        ((self.0 & 0xFF_FF00_0000_0000_0000_0000) >> 72) as u64
    }

    pub const fn address_width(&self) -> AddressWidth {
        // Bit 66-64
        let value = ((self.0 & 0x7_0000_0000_0000_0000) >> 64) as u64;
        match value {
            1 => AddressWidth::Level3PageTable,
            2 => AddressWidth::Level4PageTable,
            3 => AddressWidth::Level5PageTable,
            _ => AddressWidth::Reserved,
        }
    }

    /// Gets the second stage page translation pointer.
    ///
    /// This function will not right shift the value after the `and` operation.
    pub const fn second_stage_pointer(&self) -> u64 {
        // Bit 63~12
        (self.0 & 0xFFFF_FFFF_FFFF_F000) as u64
    }

    /// This field is applicable only for requests-without-PASID, as hardware blocks all requests-with
    /// PASID in legacy mode before they can use context table
    pub const fn translation_type(&self) -> u64 {
        // Bit 3~2
        ((self.0 & 0b1100) >> 2) as u64
    }

    /// Whether need to record/report qualified non-recoverable faults.
    pub const fn need_fault_process(&self) -> bool {
        // Bit 1
        (self.0 & 0b10) == 0
    }

    pub const fn is_present(&self) -> bool {
        // Bit 0
        (self.0 & 0b1) != 0
    }
}

#[derive(Debug)]
pub enum AddressWidth {
    /// 000b, 100b~111b
    Reserved,
    /// 001b
    Level3PageTable,
    /// 010b
    Level4PageTable,
    /// 011b
    Level5PageTable,
}

pub struct ContextTable {
    /// Total 32 devices, each device has 8 functions.
    entries_frame: Frame<()>,
    page_tables: BTreeMap<Paddr, PageTable<IommuPtConfig>>,
}

impl ContextTable {
    fn new() -> Self {
        Self {
            entries_frame: FrameAllocOptions::new().alloc_frame().unwrap(),
            page_tables: BTreeMap::new(),
        }
    }

    fn paddr(&self) -> Paddr {
        self.entries_frame.start_paddr()
    }

    fn get_or_create_page_table(
        &mut self,
        device: PciDeviceLocation,
    ) -> &mut PageTable<IommuPtConfig> {
        let bus_entry = self
            .entries_frame
            .read_val::<ContextEntry>(
                (device.device as usize * 8 + device.function as usize) * size_of::<ContextEntry>(),
            )
            .unwrap();

        if !bus_entry.is_present() {
            let table = PageTable::<IommuPtConfig>::empty();
            let address = table.root_paddr();
            self.page_tables.insert(address, table);
            let entry = ContextEntry(address as u128 | 3 | 0x1_0000_0000_0000_0000);
            self.entries_frame
                .write_val::<ContextEntry>(
                    (device.device as usize * 8 + device.function as usize)
                        * size_of::<ContextEntry>(),
                    &entry,
                )
                .unwrap();
            self.page_tables.get_mut(&address).unwrap()
        } else {
            self.page_tables
                .get_mut(&(bus_entry.second_stage_pointer() as usize))
                .unwrap()
        }
    }

    /// # Safety
    ///
    /// User must ensure that the given physical address is valid.
    unsafe fn map(
        &mut self,
        device: PciDeviceLocation,
        daddr: Daddr,
        paddr: Paddr,
    ) -> Result<(), ContextTableError> {
        if device.device >= 32 || device.function >= 8 {
            return Err(ContextTableError::InvalidDeviceId);
        }

        trace!(
            "Mapping Daddr: {:x?} to Paddr: {:x?} for device: {:x?}",
            daddr,
            paddr,
            device
        );

        let from = daddr..daddr + PAGE_SIZE;
        let prop = PageProperty {
            has_map: true,
            flags: PageFlags::RW,
            cache: CachePolicy::Uncacheable,
            priv_flags: PrivFlags::empty(),
        };

        let pt = self.get_or_create_page_table(device);
        let preempt_guard = disable_preempt();
        let mut cursor = pt.cursor_mut(&preempt_guard, &from).unwrap();

        // SAFETY: The safety is upheld by the caller.
        unsafe { cursor.map((paddr, 1, prop)).unwrap() };

        Ok(())
    }

    fn unmap(&mut self, device: PciDeviceLocation, daddr: Daddr) -> Result<(), ContextTableError> {
        if device.device >= 32 || device.function >= 8 {
            return Err(ContextTableError::InvalidDeviceId);
        }

        trace!("Unmapping Daddr: {:x?} for device: {:x?}", daddr, device);

        let pt = self.get_or_create_page_table(device);
        let preempt_guard = disable_preempt();
        let mut cursor = pt
            .cursor_mut(&preempt_guard, &(daddr..daddr + PAGE_SIZE))
            .unwrap();

        // SAFETY: This unmaps a page from the context table, which is always safe.
        let frag = unsafe { cursor.take_next(PAGE_SIZE) };
        debug_assert!(frag.is_some());

        Ok(())
    }
}
