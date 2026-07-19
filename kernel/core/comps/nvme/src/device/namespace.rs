// SPDX-License-Identifier: MPL-2.0

use aster_block::SECTOR_SIZE;

#[derive(Debug)]
pub(super) struct NvmeNamespace {
    /// Namespace ID reported by the controller.
    pub id: u32,
    /// Total number of logical blocks (NSZE).
    pub nsze: u64,
}

/// Logical block size in bytes (`2^LBADS` from the active LBA format).
///
/// For now this must match [`SECTOR_SIZE`] so the block layer and NVMe LBAs align.
pub(super) const LBA_SIZE: usize = SECTOR_SIZE;
