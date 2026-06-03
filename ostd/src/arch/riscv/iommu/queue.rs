// SPDX-License-Identifier: MPL-2.0

//! In-memory queue infrastructure for the RISC-V IOMMU.

use crate::mm::{FrameAllocOptions, HasPaddr, PAGE_SIZE, Paddr, Segment, VmIo};

/// A circular buffer of fixed-size entries stored in a dedicated page.
pub(super) struct Queue<const ENTRY_SIZE: usize> {
    segment: Segment<()>,
    capacity: usize,
    tail: usize,
}

impl<const ENTRY_SIZE: usize> Queue<ENTRY_SIZE> {
    /// Creates a queue backed by a single page.
    pub(super) fn new() -> Self {
        let segment = FrameAllocOptions::new().alloc_segment(1).unwrap();
        let capacity = PAGE_SIZE / ENTRY_SIZE;
        // The IOMMU requires the queue to have at least 2 entries and the
        // size must be a power of two so log2sz_minus_1() is well-defined.
        debug_assert!(capacity >= 2 && capacity.is_power_of_two());
        Self {
            segment,
            capacity,
            tail: 0,
        }
    }

    /// Appends an entry at the tail, wrapping around if at capacity.
    ///
    /// TODO: Check the hardware's head register (e.g. `cqh`) for available
    /// space before writing when concurrent command submission (map/unmap
    /// IOTLB invalidation) is added. Currently, only `init()` pushes a single
    /// IOFENCE command, so overwriting is impossible.
    pub(super) fn push(&mut self, entry: &[u8; ENTRY_SIZE]) {
        if self.tail >= self.capacity {
            self.tail = 0;
        }
        self.segment
            .write_val(self.tail * ENTRY_SIZE, entry)
            .unwrap();
        self.tail += 1;
    }

    /// Returns the current tail index (the number of entries pushed modulo
    /// the queue capacity). The IOMMU compares this against its internal
    /// head index and wraps independently, so the ring wraparound is
    /// expected.
    pub(super) fn tail(&self) -> usize {
        self.tail
    }

    /// Returns the physical address of the queue memory (for `cqb`/`fqb`/`pqb`).
    pub(super) fn base_paddr(&self) -> Paddr {
        self.segment.paddr()
    }

    /// Returns `log2(queue size) - 1`, the format required by the queue base
    /// register's LOG2SZ field. The caller must guarantee `capacity` is a
    /// power of two and >= 2.
    pub(super) fn log2sz_minus_1(&self) -> u8 {
        (self.capacity.ilog2() - 1) as u8
    }
}

/// A command queue entry.
pub(super) type CmdEntry = [u8; 16];

/// A fault queue entry.
pub(super) type FaultEntry = [u8; 32];

// TODO: The func3 field and operand bit positions (PR/PW, DV/DID,
// GV/AV/PSCV/PSCID, ADDR) within DW0–DW3 must be verified against
// the command format diagram before hardware testing.

/// Constructs an `IOFENCE.C` command descriptor.
pub(super) fn cmd_iofence_c(pr: bool, pw: bool) -> CmdEntry {
    let mut entry = [0u8; 16];
    let opcode: u32 = 0x02; // IOFENCE
    let func3: u32 = 0x0; // C
    let pr_bit = if pr { 1u32 << 15 } else { 0 };
    let pw_bit = if pw { 1u32 << 14 } else { 0 };
    entry[0..4].copy_from_slice(&(opcode | func3 | pr_bit | pw_bit).to_le_bytes());
    entry
}

/// Constructs an `IODIR.INVAL_DDT` command descriptor.
pub(super) fn cmd_iodir_inval_ddt(dv: bool, device_id: u16) -> CmdEntry {
    let mut entry = [0u8; 16];
    let opcode: u32 = 0x03; // IODIR
    let dv_bit = if dv { 1u32 << 15 } else { 0 };
    let dw0 = opcode | dv_bit | (device_id as u32 & 0xFFFFFF);
    entry[0..4].copy_from_slice(&dw0.to_le_bytes());
    entry
}

/// Constructs an `IOTINVAL.VMA` command descriptor.
#[expect(dead_code)]
pub(super) fn cmd_iotinval_vma(gv: bool, addr: u64, pscid: u32) -> CmdEntry {
    let mut entry = [0u8; 16];
    let opcode: u32 = 0x01; // IOTINVAL
    let func3: u32 = 0x0; // VMA
    let gv_bit = if gv { 1u32 << 15 } else { 0 };
    let av_bit = 1u32 << 14; // address valid
    let dw0 = opcode | func3 | gv_bit | av_bit | (pscid & 0xFFFFF);
    entry[0..4].copy_from_slice(&dw0.to_le_bytes());
    entry[4..8].copy_from_slice(&((addr >> 12) as u32).to_le_bytes());
    entry[8..12].copy_from_slice(&((addr >> 44) as u32).to_le_bytes());
    entry
}

/// Constructs an `IOTINVAL.GVMA` command descriptor.
// TODO: The `func3` value for GVMA (0x1) is tentative pending spec verification.
// GVMA is a distinct `func3` from VMA (0x0) within the IOTINVAL opcode.
#[expect(dead_code)]
pub(super) fn cmd_iotinval_gvma(gv: bool, av: bool, gscid: u16, addr: u64) -> CmdEntry {
    let mut entry = [0u8; 16];
    let opcode: u32 = 0x01; // IOTINVAL
    let func3: u32 = 0x1; // GVMA
    let gv_bit = if gv { 1u32 << 15 } else { 0 };
    let av_bit = if av { 1u32 << 14 } else { 0 };
    let dw0 = opcode | func3 | gv_bit | av_bit | ((gscid as u32) & 0xFFFF);
    entry[0..4].copy_from_slice(&dw0.to_le_bytes());
    entry[4..8].copy_from_slice(&((addr >> 12) as u32).to_le_bytes());
    entry[8..12].copy_from_slice(&((addr >> 44) as u32).to_le_bytes());
    entry
}
