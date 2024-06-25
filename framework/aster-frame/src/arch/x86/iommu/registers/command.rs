// SPDX-License-Identifier: MPL-2.0

use bitflags::bitflags;

bitflags! {
    /// Global Command to enable functions in IOMMU. All field is write-only.
    pub struct GlobalCommand: u32{
        /// Compatibility Format Interrupt, only valid if interrupt-remapping is supported.
        ///
        /// Interrupt remapping will block compatibility format interrupts if set to 0.
        /// Otherwise these interrupts will bypass interrupt remapping.
        const CFI =         1 << 23;
        /// Set Interrupt Remap Table Pointer, only valid if interrupt-remapping is supported.
        ///
        /// Software sets this filed to set/update the interrupt remapping table pointer used
        /// by hardware. The interrupt remapping table pointer is specified through the Interrupt
        /// Remapping Table Address (IRTA_REG) register.
        const SIRTP =       1 << 24;
        /// Interrupt Remapping Enable, only valid if hardware support interrupt remapping.
        /// Set to 1 if enable interrupt-remapping hardware.
        ///
        /// Hardware reports the status of the interrupt remapping enable operation through the
        /// IRES field in the Global Status register.
        const IRE =         1 << 25;
        /// Queued Invalidation Enable, only valid if hardware support queued invalidations.
        /// Set to 1 to enable use of queued validations.
        ///
        /// Hardware reports the status of queued invalidation enable operation through QIES
        /// field in Global Status register.
        const QIE =         1 << 26;
        /// Write Buffer Flush, only valid for implementations requiring write buffer flushing.
        ///
        /// Software sets this field to request that hardware flush the Root-Complex
        /// internal write buffers. This is done to ensure any updates to the memory resident
        /// remapping structures are not held in any internal write posting buffers.
        ///
        /// Hardware reports the status of the write buffer flushing operation through WBFS
        /// field in Global Status register.
        const WBF =         1 << 27;
        /// Set Root Table Pointer.
        ///
        /// Software sets this field to set/update the root-table pointer (and translation
        /// table mode) used by hardware. The root-table pointer (and translation table
        /// mode) is specified through the Root Table Address (RTADDR_REG) register.
        const SRTP =        1 << 30;
        /// Translation Enable.
        ///
        /// Software writes to this field to request hardware to enable/disable DMA remapping.
        ///
        /// 0: Disable DMA remapping; 1: Enable DMA remapping.
        ///
        /// Hardware reports the status of the translation enable operation through TES field
        /// in the Global Status register.
        const TE =          1 << 31;
    }
}
