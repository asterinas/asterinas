// SPDX-License-Identifier: MPL-2.0

//! Device Directory Table (DDT) for the RISC-V IOMMU.

use alloc::collections::BTreeMap;

use super::second_stage::IommuPtConfig;
use crate::mm::{Frame, FrameAllocOptions, HasPaddr, Paddr, PageTable, VmIo};

// A non-leaf DDT entry.
//
// Per the RISC-V IOMMU spec: V at bit 0, 11 reserved bits, PPN at bits 55:12,
// and 8 reserved bits at the top. QEMU's emulation places PPN at bits 53:10
// instead; the `riscv_iommu_qemu_quirk` feature (opt-in) enables the shift=10
// encoding.
#[repr(C)]
#[derive(Clone, Copy, Pod)]
struct DdtEntry(u64);

impl DdtEntry {
    #[cfg(feature = "riscv_iommu_qemu_quirk")]
    const PPN_SHIFT: u32 = 10;
    #[cfg(not(feature = "riscv_iommu_qemu_quirk"))]
    const PPN_SHIFT: u32 = 12;
    const PPN_MASK: u64 = 0xFFFFFFFFFFF; // 44-bit PPN

    fn is_valid(&self) -> bool {
        (self.0 & 0x1) != 0
    }

    fn paddr(&self) -> Paddr {
        (((self.0 >> Self::PPN_SHIFT) & Self::PPN_MASK) as usize) << 12
    }

    fn new(paddr: Paddr) -> Self {
        let ppn = (paddr >> 12) as u64;
        Self((ppn << Self::PPN_SHIFT) | 0x1)
    }
}

// Size in bytes of a Base-format Device Context.
const DC_SIZE: usize = 32;

// The `MODE` field value for Sv39x4 in `iohgatp` when `fctl.GXL=0`.
const IOHGATP_MODE_SV39X4: u64 = 8;

/// Errors that can occur during DDT manipulation.
#[derive(Debug)]
pub(super) enum DdtError {
    /// A write to the DDT page failed.
    ModificationError,
}

/// Device Directory Table for translating `device_id` to a Device Context.
pub(super) struct DdtTable {
    root_frame: Frame<()>,
    leaf_frames: BTreeMap<Paddr, Frame<()>>,
}

impl DdtTable {
    // Entries per root table page (2LVL, base format: 9 bits for DDI[1]).
    const ROOT_ENTRIES: usize = 1 << 9;
    // Entries per leaf table page (base format: 7 bits for DDI[0]).
    const LEAF_ENTRIES: usize = 1 << 7;

    pub(super) fn new() -> Self {
        Self {
            root_frame: FrameAllocOptions::new().zeroed(true).alloc_frame().unwrap(),
            leaf_frames: BTreeMap::new(),
        }
    }

    /// Returns the root table physical address.
    pub(super) fn root_paddr(&self) -> Paddr {
        self.root_frame.paddr()
    }

    // Returns the leaf table for `ddi1`, allocating a new zeroed page if absent.
    fn get_or_create_leaf(&mut self, ddi1: usize) -> Result<&Frame<()>, DdtError> {
        let root_offset = ddi1 * size_of::<DdtEntry>();
        let root_entry = self
            .root_frame
            .read_val::<DdtEntry>(root_offset)
            .map_err(|_| DdtError::ModificationError)?;

        if root_entry.is_valid() {
            let paddr = root_entry.paddr();
            return Ok(self.leaf_frames.get(&paddr).unwrap());
        }

        let leaf_frame = FrameAllocOptions::new()
            .zeroed(true)
            .alloc_frame()
            .map_err(|_| DdtError::ModificationError)?;
        let paddr = leaf_frame.paddr();
        let entry = DdtEntry::new(paddr);
        self.root_frame
            .write_val(root_offset, &entry)
            .map_err(|_| DdtError::ModificationError)?;
        self.leaf_frames.insert(paddr, leaf_frame);
        Ok(self.leaf_frames.get(&paddr).unwrap())
    }

    /// Writes a Device Context for `device_id` pointing at the given page table.
    pub(super) fn enable_device(
        &mut self,
        device_id: u16,
        page_table: &PageTable<IommuPtConfig>,
    ) -> Result<(), DdtError> {
        let ddi0 = (device_id & 0x7F) as usize;
        let ddi1 = ((device_id >> 7) & 0x1FF) as usize;

        let leaf = self.get_or_create_leaf(ddi1)?;
        let dc_offset = ddi0 * DC_SIZE;

        // GSCID is left at 0 because per-VM invalidation is not yet
        // implemented; all VMs share one invalidation domain.
        let root_paddr = page_table.root_paddr();
        let ppn = (root_paddr >> 12) as u64;
        let iohgatp: u64 = (IOHGATP_MODE_SV39X4 << 60) | ppn;
        leaf.write_val(dc_offset + 8, &iohgatp)
            .map_err(|_| DdtError::ModificationError)?;

        let tc: u64 = 0x1;
        leaf.write_val(dc_offset, &tc)
            .map_err(|_| DdtError::ModificationError)?;

        // TODO: Issue `IODIR.INVAL_DDT` with DV=1 for the affected `device_id`
        // after writing a leaf DDT entry. Currently the caller (`init`) issues a
        // blanket `IOFENCE.C` instead, which is coarser but sufficient.

        Ok(())
    }
}
