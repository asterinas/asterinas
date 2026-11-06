// SPDX-License-Identifier: MPL-2.0

//! NVMe protocol data structures.
//!
//! Refer to NVM Express Base Specification Revision 2.0, Section 3.3.3

use aster_util::{field_ptr, safe_ptr::SafePtr};
use ostd::mm::dma::DmaCoherent;

/// Submission Queue Entry (SQE).
///
/// See NVMe Spec 2.0, Section 3.3.1 (Submission Queue Entry).
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(crate) struct NvmeCommand {
    /// Opcode.
    opcode: u8,
    /// Flags.
    flags: u8,
    /// Command ID.
    cid: u16,
    /// Namespace identifier.
    nsid: u32,
    /// Reserved.
    _rsvd: u64,
    /// Metadata pointer.
    mptr: u64,
    /// Data pointer.
    dptr: [u64; 2],
    /// Command dword 10.
    cdw10: u32,
    /// Command dword 11.
    cdw11: u32,
    /// Command dword 12.
    cdw12: u32,
    /// Command dword 13.
    cdw13: u32,
    /// Command dword 14.
    cdw14: u32,
    /// Command dword 15.
    cdw15: u32,
}

/// Completion Queue Entry (CQE).
///
/// See NVMe Spec 2.0, Section 3.3.3.2 (Common Completion Queue Entry), Figure 89.
/// Layout by dword (offsets0–15 in the entry):
/// - **Dword 0**: Command specific.
/// - **Dword 1**: Command specific.
/// - **Dword 2**: SQ Head Pointer (bits 15:0) | SQ Identifier (bits 31:16).
/// - **Dword 3**: Command Identifier (bits 15:0) | Phase Tag (bit 16) | Status Field (bits 31:17, 15 bits).
///
/// The Status bits are further defined in Figure 92 (DNR, M, CRD, SCT, SC, etc.).
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(crate) struct NvmeCompletion {
    /// Dword 0: Command Specific (32 bits).
    dword0: u32,

    /// Dword 1: Command Specific (32 bits).
    dword1: u32,

    /// Dword 2, bits 0-15: SQ Head Pointer (16 bits).
    ///
    /// The head pointer of the corresponding Submission Queue that is updated
    /// by the controller when this entry is placed into the Completion Queue.
    sq_head: u16,

    /// Dword 2, bits 16-31: SQ Identifier (16 bits).
    ///
    /// The Submission Queue identifier that is associated with this completion.
    sq_id: u16,

    /// Dword 3, bits 0-15: Command Identifier (16 bits).
    ///
    /// The Command Identifier (CID) of the command that this completion is associated with.
    cid: u16,

    /// Dword 3, bits 16-31: Status Field (16 bits).
    status: u16,
}

impl NvmeCommand {
    /// Creates a command entry from fully encoded command fields.
    ///
    /// `cdw` supplies command dwords starting at CDW10; any of CDW11–CDW15 not included are zero.
    /// There can be at most six command dwords (i.e., `N <= 6`).
    pub(crate) fn from_raw_fields<const N: usize>(
        opcode: u8,
        flags: u8,
        nsid: u32,
        dptr: [u64; 2],
        cdw: [u32; N],
    ) -> Self {
        const { assert!(N <= 6) };
        Self {
            opcode,
            flags,
            cid: 0,
            nsid,
            _rsvd: 0,
            mptr: 0,
            dptr,
            cdw10: cdw.first().copied().unwrap_or(0),
            cdw11: cdw.get(1).copied().unwrap_or(0),
            cdw12: cdw.get(2).copied().unwrap_or(0),
            cdw13: cdw.get(3).copied().unwrap_or(0),
            cdw14: cdw.get(4).copied().unwrap_or(0),
            cdw15: cdw.get(5).copied().unwrap_or(0),
        }
    }

    /// Sets the Command Identifier (CID) for this submission queue entry.
    pub(crate) fn set_cid(&mut self, cid: u16) {
        self.cid = cid;
    }
}

impl NvmeCompletion {
    /// Reads the phase tag (P) from a completion queue slot.
    pub(crate) fn read_phase_tag(ring_slot_ptr: &SafePtr<NvmeCompletion, &DmaCoherent>) -> bool {
        let status = field_ptr!(ring_slot_ptr, NvmeCompletion, status)
            .read_once()
            .expect("CQ status field must be valid within allocated DMA ring");
        (status & 1) != 0
    }

    /// Returns the completion entry SQ head pointer.
    pub(crate) fn sq_head(&self) -> u16 {
        self.sq_head
    }

    /// Returns the completion entry SQ identifier.
    pub(crate) fn sq_id(&self) -> u16 {
        self.sq_id
    }

    /// Returns the completion entry command identifier.
    pub(crate) fn cid(&self) -> u16 {
        self.cid
    }

    /// Returns the raw completion status bits.
    pub(crate) fn status(&self) -> u16 {
        self.status
    }

    /// Masks Status Code (SC), DW3 bits 24:17, in bits 8:1 of `status`.
    const STATUS_SC_MASK: u16 = 0x01FE;

    /// Checks if the completion indicates an error.
    ///
    /// Returns `true` if the Status Code is non-zero.
    pub(crate) fn has_error(&self) -> bool {
        self.status_code() != 0
    }

    /// Gets the Status Code from the completion status field.
    ///
    /// Returns the Status Code (SC).
    pub(crate) fn status_code(&self) -> u8 {
        ((self.status & Self::STATUS_SC_MASK) >> 1) as u8
    }
}
