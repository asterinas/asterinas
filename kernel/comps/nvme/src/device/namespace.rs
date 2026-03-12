// SPDX-License-Identifier: MPL-2.0

#[derive(Debug)]
pub(crate) struct NvmeNamespace {
    /// Namespace ID reported by the controller.
    pub(crate) id: u32,
    /// Total number of logical blocks (NSZE).
    pub(crate) nsze: u64,
    /// Logical block size in bytes (2^LBADS from the active LBA format).
    pub(crate) lba_size: u64,
}
