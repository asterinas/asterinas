use bitflags::bitflags;
use log::debug;
use spin::Once;
use volatile::{
    access::{ReadOnly, ReadWrite, WriteOnly},
    Volatile,
};

use crate::{
    arch::{
        iommu::fault,
        x86::kernel::acpi::{
            dmar::{Dmar, Remapping},
            ACPI_TABLES,
        },
    },
    vm::paddr_to_vaddr,
};

use super::{context_table::RootTable, IommuError};

#[derive(Debug)]
pub struct RemappingRegisters {
    version: Volatile<&'static u32, ReadOnly>,
    capability: Volatile<&'static u64, ReadOnly>,
    extended_capability: Volatile<&'static u64, ReadOnly>,
    global_command: Volatile<&'static mut u32, WriteOnly>,
    global_status: Volatile<&'static u32, ReadOnly>,
    root_table_address: Volatile<&'static mut u64, ReadWrite>,
    context_command: Volatile<&'static mut u64, ReadWrite>,
}

impl RemappingRegisters {
    pub fn capability(&self) -> Capability {
        Capability::from_bits_truncate(self.capability.read())
    }

    /// Create a instance from base address
    fn new(root_table: &RootTable) -> Option<Self> {
        let dmar = Dmar::new()?;
        let acpi_table_lock = ACPI_TABLES.get().unwrap().lock();

        debug!("DMAR:{:#x?}", dmar);
        let base_address = {
            let mut addr = 0;
            for remapping in dmar.remapping_iter() {
                if let Remapping::Drhd(drhd) = remapping {
                    addr = drhd.register_base_addr()
                }
            }
            if addr == 0 {
                panic!("There should be a DRHD structure in the DMAR table");
            }
            addr
        };

        let vaddr: usize = paddr_to_vaddr(base_address as usize);
        // Safety: All offsets and sizes are strictly adhered to in the manual, and the base address is obtained from Drhd.
        let mut remapping_reg = unsafe {
            fault::init(vaddr);
            let version = Volatile::new_read_only(&*(vaddr as *const u32));
            let capability = Volatile::new_read_only(&*((vaddr + 0x08) as *const u64));
            let extended_capability: Volatile<&u64, ReadOnly> =
                Volatile::new_read_only(&*((vaddr + 0x10) as *const u64));
            let global_command = Volatile::new_write_only(&mut *((vaddr + 0x18) as *mut u32));
            let global_status = Volatile::new_read_only(&*((vaddr + 0x1C) as *const u32));
            let root_table_address = Volatile::new(&mut *((vaddr + 0x20) as *mut u64));
            let context_command = Volatile::new(&mut *((vaddr + 0x28) as *mut u64));
            Self {
                version,
                capability,
                extended_capability,
                global_command,
                global_status,
                root_table_address,
                context_command,
            }
        };

        // write remapping register
        remapping_reg
            .root_table_address
            .write(root_table.paddr() as u64);
        // start writing
        remapping_reg.global_command.write(0x4000_0000);
        // wait until complete
        while remapping_reg.global_status.read() & 0x4000_0000 == 0 {}

        // enable iommu
        remapping_reg.global_command.write(0x8000_0000);

        debug!("IOMMU registers:{:#x?}", remapping_reg);

        Some(remapping_reg)
    }
}

bitflags! {
    pub struct Capability : u64{
        /// Number of domain support.
        ///
        /// ```norun
        /// 0 => 4-bit domain-ids with support for up to 16 domains.
        /// 1 => 6-bit domain-ids with support for up to 64 domains.
        /// 2 => 8-bit domain-ids with support for up to 256 domains.
        /// 3 => 10-bit domain-ids with support for up to 1024 domains.
        /// 4 => 12-bit domain-ids with support for up to 4K domains.
        /// 5 => 14-bit domain-ids with support for up to 16K domains.
        /// 6 => 16-bit domain-ids with support for up to 64K domains.
        /// 7 => Reserved.
        /// ```
        const ND =          0x7;
        /// Required Write-Buffer Flushing.
        const RWBF =        1 << 4;
        /// Protected Low-Memory Region
        const PLMR =        1 << 5;
        /// Protected High-Memory Region
        const PHMR =        1 << 6;
        /// Caching Mode
        const CM =          1 << 7;
        /// Supported Adjusted Guest Address Widths.
        /// ```norun
        /// 0/4 => Reserved
        /// 1   => 39-bit AGAW (3-level page-table)
        /// 2   => 48-bit AGAW (4-level page-table)
        /// 3   => 57-bit AGAW (5-level page-table)
        /// ```
        const SAGAW =       0x1F << 8;
        /// Maximum Guest Address Width.
        /// The maximum guest physical address width supported by second-stage translation in remapping hardware.
        /// MGAW is computed as (N+1), where N is the valued reported in this field.
        const MGAW =        0x3F << 16;
        /// Zero Length Read. Whether the remapping hardware unit supports zero length
        /// DMA read requests to write-only pages.
        const ZLR =         1 << 22;
        /// Fault-recording Register offset, specifies the offset of the first fault recording register
        /// relative to the register base address of this remapping hardware unit.
        ///
        /// If the register base address is X, and the value reported in this field
        /// is Y, the address for the first fault recording register is calculated as X+(16*Y).
        const FRO =         0x3FF << 24;
        /// Second Stage Large Page Support.
        /// ```norun
        /// 2/3 => Reserved
        /// 0   => 21-bit offset to page frame(2MB)
        /// 1   => 30-bit offset to page frame(1GB)
        /// ```
        const SSLPS =       0xF << 34;
        /// Page Selective Invalidation. Whether hardware supports page-selective invalidation for IOTLB.
        const PSI =         1 << 39;
        /// Number of Fault-recording Registers. Number of fault recording registers is computed as N+1.
        const NFR =         0xFF << 40;
        /// Maximum Address Mask Value,  indicates the maximum supported value for the
        /// Address Mask (AM) field in the Invalidation Address register
        /// (IVA_REG), and IOTLB Invalidation Descriptor (iotlb_inv_dsc) used
        /// for invalidations of second-stage translation.
        const MAMV =        0x3F << 48;
        /// Write Draining.
        const DWD =         1 << 54;
        /// Read Draining.
        const DRD =         1 << 55;
        /// First Stage 1-GByte Page Support.
        const FS1GP =       1 << 56;
        /// Posted Interrupts Support.
        const PI =          1 << 59;
        /// First Stage 5-level Paging Support.
        const FS5LP =       1 << 60;
        /// Enhanced Command Support.
        const ECMDS =       1 << 61;
        /// Enhanced Set Interrupt Remap Table Pointer Support.
        const ESIRTPS =     1 << 62;
        /// Enhanced Set Root Table Pointer Support.
        const ESRTPS =      1 << 63;
    }
}

pub static REMAPPING_REGS: Once<RemappingRegisters> = Once::new();

pub(super) fn init(root_table: &RootTable) -> Result<(), IommuError> {
    let remapping_regs = RemappingRegisters::new(root_table).ok_or(IommuError::NoIommu)?;
    REMAPPING_REGS.call_once(|| remapping_regs);
    Ok(())
}
