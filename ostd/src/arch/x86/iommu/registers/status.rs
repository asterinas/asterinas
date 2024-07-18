// SPDX-License-Identifier: MPL-2.0

use bitflags::bitflags;

bitflags! {
    /// Global Status of the IOMMU. All fields is read-only. Some description of the fields
    /// is related to the fields in `GlobalCommand`.
    pub struct GlobalStatus: u32{
        /// Compatibility Format Interrupt Status. The value reported in this field is
        /// applicable only when interrupt-remapping is enabled and extended interrupt
        /// model (x2APIC) mode is not enables.
        ///
        /// - 0: Compatibility format interrupts are blocked.
        /// - 1: Compatibility format interrupts are processed as pass-through (bypassing
        /// interrupt remapping).
        const CFIS =        1 << 23;
        /// Interrupt Remapping Table Pointer Status.
        ///
        /// This field is cleared by hardware when software sets the SIRTP field in the Global
        /// Command register. This field is Set by hardware when hardware completes the
        /// `Set Interrupt Remap Table Pointer` operation using the value provided in the
        /// Interrupt Remapping Table Address register.
        const IRTPS =       1 << 24;
        /// Interrupt Remapping Enable Status.
        ///
        /// - 0: Interrupt-remapping hardware is not enabled.
        /// - 1: Interrupt-remapping hardware is enabled.
        const IRES =        1 << 25;
        /// Queued Invalidation Enable Status.
        ///
        /// - 0: queued invalidation is not enabled.
        /// - 1: queued invalidation is enabled.
        const QIES =        1 << 26;
        /// Write Buffer Flush Status. This field is valid only for implementations requiring
        /// write buffer flushing. This field indicates the status of the write buffer flush
        /// command.
        ///
        /// - Set by hardware when software sets the WBF field in the Global Command register.
        /// - Cleared by hardware when hardware completes the write buffer flushing operation.
        const WBFS =        1 << 27;
        /// Root Table Pointer Status.
        ///
        /// This field is cleared by hardware when software sets the SRTP field in the Global
        /// Command register. This field is set by hardware when hardware completes the
        /// `Set Root Table Pointer`` operation using the value provided in the Root Table
        /// Address register.
        const RTPS =        1 << 30;
        /// Translation Enable Status.
        ///
        /// - 0: DMA remapping is not enabled.
        /// - 1: DMA remapping is enabled.
        const TES =         1 << 31;
    }
}
