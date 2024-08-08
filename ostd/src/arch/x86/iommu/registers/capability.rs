// SPDX-License-Identifier: MPL-2.0

use core::fmt::Debug;

use bitflags::bitflags;

/// Capability in IOMMU.
pub struct Capability(u64);

impl Capability {
    /// Create Capability from `value`
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Capability flags
    pub const fn flags(&self) -> CapabilityFlags {
        CapabilityFlags::from_bits_truncate(self.0)
    }

    /// Number of Fault-recording. The maximum number of fault recording registers per
    /// remapping hardware unit is 256.
    ///
    /// Number of fault recording registers is computed as N+1, where N is the value
    /// reported in this field.
    pub const fn fault_recording_number(&self) -> u64 {
        const NFR_MASK: u64 = 0xFF << 40;
        (self.0 & NFR_MASK) >> 40
    }

    /// Maximum Address Mask Value, indicates the maximum supported value for them Address
    /// Mask (AM) field in the Invalidation Address register (IVA_REG), and IOTLB Invalidation
    /// Descriptor (iotlb_inv_dsc) used for invalidations of second-stage translation.
    pub const fn maximum_address_mask_value(&self) -> u64 {
        const MAMV_MASK: u64 = 0x3F << 48;
        (self.0 & MAMV_MASK) >> 48
    }

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
    pub const fn domain_support_number(&self) -> u64 {
        const ND_MASK: u64 = 0x7;
        self.0 & ND_MASK
    }

    /// Supported Adjusted Guest Address Widths.
    /// ```norun
    /// 0/4 => Reserved
    /// 1   => 39-bit AGAW (3-level page-table)
    /// 2   => 48-bit AGAW (4-level page-table)
    /// 3   => 57-bit AGAW (5-level page-table)
    /// ```
    pub const fn supported_adjusted_guest_address_widths(&self) -> u64 {
        const SAGAW_MASK: u64 = 0x1F << 8;
        (self.0 & SAGAW_MASK) >> 8
    }

    /// Fault-recording Register offset, specifies the offset of the first fault recording
    /// register relative to the register base address of this remapping hardware unit.
    ///
    /// If the register base address is X, and the value reported in this field
    /// is Y, the address for the first fault recording register is calculated as X+(16*Y).
    pub const fn fault_recording_register_offset(&self) -> u64 {
        const FRO_MASK: u64 = 0x3FFF << 24;
        (self.0 & FRO_MASK) >> 24
    }
    /// Second Stage Large Page Support.
    /// ```norun
    /// 2/3 => Reserved
    /// 0   => 21-bit offset to page frame(2MB)
    /// 1   => 30-bit offset to page frame(1GB)
    /// ```
    pub const fn second_stage_large_page_support(&self) -> u64 {
        const SSLPS_MASK: u64 = 0xF << 34;
        (self.0 & SSLPS_MASK) >> 34
    }

    /// Maximum Guest Address Width. The maximum guest physical address width supported
    /// by second-stage translation in remapping hardware.
    /// MGAW is computed as (N+1), where N is the valued reported in this field.
    pub const fn maximum_guest_address_width(&self) -> u64 {
        const MGAW_MASK: u64 = 0x3F << 16;
        (self.0 & MGAW_MASK) >> 16
    }
}

impl Debug for Capability {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Capability")
            .field("flags", &self.flags())
            .field(
                "maximum_guest_address_width",
                &self.maximum_guest_address_width(),
            )
            .field(
                "second_stage_large_page_support",
                &self.second_stage_large_page_support(),
            )
            .field(
                "fault_recording_register_offset",
                &self.fault_recording_register_offset(),
            )
            .field(
                "supported_adjusted_guest_address_widths",
                &self.supported_adjusted_guest_address_widths(),
            )
            .field("domain_support_number", &self.domain_support_number())
            .field(
                "maximum_address_mask_value",
                &self.maximum_address_mask_value(),
            )
            .field("fault_recording_number", &self.fault_recording_number())
            .field("raw", &self.0)
            .finish()
    }
}

bitflags! {
    /// Capability flags in IOMMU.
    pub struct CapabilityFlags: u64{
        /// Required Write-Buffer Flushing.
        const RWBF =        1 << 4;
        /// Protected Low-Memory Region
        const PLMR =        1 << 5;
        /// Protected High-Memory Region
        const PHMR =        1 << 6;
        /// Caching Mode
        const CM =          1 << 7;
        /// Zero Length Read. Whether the remapping hardware unit supports zero length DMA
        /// read requests to write-only pages.
        const ZLR =         1 << 22;
        /// Page Selective Invalidation. Whether hardware supports page-selective invalidation
        /// for IOTLB.
        const PSI =         1 << 39;
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
