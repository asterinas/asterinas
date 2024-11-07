// SPDX-License-Identifier: MPL-2.0

use core::fmt::Debug;

use bitflags::bitflags;

pub struct ExtendedCapability(u64);

impl ExtendedCapability {
    /// Creates ExtendedCapability from `value`
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Extended capability flags
    pub const fn flags(&self) -> ExtendedCapabilityFlags {
        ExtendedCapabilityFlags::from_bits_truncate(self.0)
    }

    /// IOTLB Register Offset. This field specifies the offset to the IOTLB registers relative
    /// to the register base address of this remapping hardware unit.
    ///
    /// If the register base address is X, and the value reported in this field is Y, the
    /// address for the IOTLB registers is calculated as X+(16*Y).
    pub const fn iotlb_register_offset(&self) -> u64 {
        const IRO_MASK: u64 = 0x3FF << 8;
        (self.0 & IRO_MASK) >> 8
    }

    /// Maximum Handle Mask Value, indicates the maximum supported value for the Interrupt
    /// Mask (IM) field in the Interrupt Entry Cache Invalidation Descriptorr (iec_inv_dsc).
    pub const fn maximum_handle_mask(&self) -> u64 {
        const MHMV_MASK: u64 = 0xF << 20;
        (self.0 & MHMV_MASK) >> 20
    }

    /// PASID Size Supported, indicates the PASID size supported by the remapping hardware
    /// for requests-with-PASID. A value of N in this field indicates hardware supports
    /// PASID field of N+1 bits.
    ///
    /// This field is unused and reported as 0 if Scalable Mode Translation Support (SMTS)
    /// field is Clear.
    pub const fn pasid_size(&self) -> u64 {
        const PSS_MASK: u64 = 0x1F << 35;
        (self.0 & PSS_MASK) >> 35
    }
}

impl Debug for ExtendedCapability {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ExtendedCapability")
            .field("flags", &self.flags())
            .field("maximum_handle_mask", &self.maximum_handle_mask())
            .field("pasid_size", &self.pasid_size())
            .field("iotlb_register_offset", &self.iotlb_register_offset())
            .field("raw", &self.0)
            .finish()
    }
}

bitflags! {
    /// Extended Capability flags in IOMMU.
    ///
    /// TODO: Add adetailed description of each flag.
    pub struct ExtendedCapabilityFlags: u64{
        /// Page-walk Conherency.
        const C =           1 << 0;
        /// Queued Invalidation Support.
        const QI =          1 << 1;
        /// Device-TLB Support.
        const DT =          1 << 2;
        /// Interrupt Remapping Support
        const IR =          1 << 3;
        /// Extended Interrupt Mode.
        const EIM =         1 << 4;
        /// Pass Through Support.
        const PT =          1 << 6;
        /// Snoop Control.
        const SC =          1 << 7;
        /// Memory Type Support.
        const MTS =         1 << 25;
        /// Nested Translation Support.
        const NEST =        1 << 26;
        /// Page Request Support.
        const PRS =         1 << 29;
        /// Execute Request Support.
        const ERS =         1 << 30;
        /// Supervisor Request Support.
        const SRS =         1 << 31;
        /// No Write Flag Support.
        const NWFS =        1 << 33;
        /// Extended Accessed Flag Support.
        const EAFS =        1 << 34;
        /// Process Address Space ID Supported.
        const PASID =       1 << 40;
        /// Device-TLB Invalidation Throttle.
        const DIT =         1 << 41;
        /// Page-request Drain Support.
        const PDS =         1 << 42;
        /// Scalable Mode Translation Support.
        const SMTS =        1 << 43;
        /// Virtual Command Support.
        const VCS =         1 << 44;
        /// Second-Stage Accessed/Dirty Support.
        const SSADS =       1 << 45;
        /// Second-stage Translation Support.
        const SSTS =        1 << 46;
        /// First-stage Translation Support.
        const FSTS =        1 << 47;
        /// Scalable-Mode Page-walk Coherency Support.
        const SMPWCS =      1 << 48;
        /// RID-PASID Support.
        const RPS =         1 << 49;
        /// Performance Monitoring Support.
        const PMS =         1 << 51;
        /// Abort DMA Mode Support.
        const ADMS =        1 << 52;
        /// RID_PRIV Support.
        const RPRIVS =      1 << 53;
        /// Stop Marker Support.
        const SMS =         1 << 58;
    }
}
