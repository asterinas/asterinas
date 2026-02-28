// SPDX-License-Identifier: MPL-2.0

//! NVMe Command and Completion structures.
//!
//! Refer to NVM Express Base Specification Revision 2.0:
//! - Section 5: Admin Command Set
//! - Section 6: NVM Command Set

/// Bit position for the FUSE (Fused Operation) field in the command flags byte.
///
/// The FUSE field (bits 6:7) indicates whether this command is part of a fused operation:
/// - 00b: Normal command (not part of a fused operation)
/// - 01b: First command of a fused operation
/// - 10b: Second command of a fused operation
/// - 11b: Reserved
const NOT_FUSED_BITS: u8 = 6;

/// Phase Tag bit mask.
///
/// Used to identify the phase of the completion queue entry.
pub(crate) const STATUS_PHASE_TAG_MASK: u16 = 0x0001;

/// Status Code and Do Not Retry bit mask.
///
/// If any of these bits are set, the command failed.
/// Status Code 0x0000 indicates success.
const STATUS_ERROR_MASK: u16 = 0xFFFE;

/// Admin Command Set opcodes.
///
/// See NVMe Spec 2.0, Section 5 (Admin Command Set).
#[repr(u8)]
enum AdminCommandSet {
    /// Delete I/O Submission Queue command. See Section 5.7.
    DeleteIOSQ = 0x00,
    /// Create I/O Submission Queue command. See Section 5.5.
    CreateIOSQ = 0x01,
    /// Delete I/O Completion Queue command. See Section 5.6.
    DeleteIOCQ = 0x04,
    /// Create I/O Completion Queue command. See Section 5.4.
    CreateIOCQ = 0x05,
    /// Identify command. See Section 5.17.
    IdentifyCommand = 0x06,
}

#[expect(dead_code)]
pub(crate) fn delete_io_completion_queue(cid: u16, qid: u16) -> NvmeCommand {
    NvmeCommand {
        opcode: AdminCommandSet::DeleteIOCQ as u8,
        flags: 0,
        cid,
        nsid: 0,
        _rsvd: 0,
        mptr: 0,
        dptr: [0, 0],
        cdw10: qid as u32,
        cdw11: 0,
        cdw12: 0,
        cdw13: 0,
        cdw14: 0,
        cdw15: 0,
    }
}

#[expect(dead_code)]
pub(crate) fn delete_io_submission_queue(cid: u16, qid: u16) -> NvmeCommand {
    NvmeCommand {
        opcode: AdminCommandSet::DeleteIOSQ as u8,
        flags: 0,
        cid,
        nsid: 0,
        _rsvd: 0,
        mptr: 0,
        dptr: [0, 0],
        cdw10: qid as u32,
        cdw11: 0,
        cdw12: 0,
        cdw13: 0,
        cdw14: 0,
        cdw15: 0,
    }
}

/// I/O Command Set opcodes (NVM Command Set).
///
/// See NVMe Spec 2.0, Section 7 (I/O Commands).
#[repr(u8)]
enum IoCommandSet {
    /// Flush command. See Section 7.1.
    Flush = 0x00,
    /// Write command. See Section 7.
    Write = 0x01,
    /// Read command. See Section 7.
    Read = 0x02,
}

/// Completion Queue Entry (CQE).
///
/// See NVMe Spec 2.0, Section 3.3.1 (Completion Queue Entry).
/// The Completion Queue Entry is 16 bytes and consists of 4 Dwords:
///
/// - **Dword 0 (bits 0-31)**: Command Specific
/// - **Dword 1 (bits 32-63)**: Reserved
/// - **Dword 2 (bits 64-95)**: SQ Head Pointer (bits 0-15) | SQ Identifier (bits 16-31)
/// - **Dword 3 (bits 96-127)**: Command Identifier (bits 0-15) | Status Field (bits 16-31)
///
/// Status Field format (16 bits):
/// - Bit 0: Phase Tag (P)
/// - Bits 1-14: Status Code (SC) - 14 bits
/// - Bit 15: Do Not Retry (DNR)
#[repr(C)]
#[derive(Debug, Copy, Clone, Pod)]
pub(crate) struct NvmeCompletion {
    /// Dword 0: Command Specific (32 bits).
    pub dword0: u32,

    /// Dword 1: Command Specific (32 bits).
    pub dword1: u32,

    /// Dword 2, bits 0-15: SQ Head Pointer (16 bits).
    ///
    /// The head pointer of the corresponding Submission Queue that is updated
    /// by the controller when this entry is placed into the Completion Queue.
    pub sq_head: u16,

    /// Dword 2, bits 16-31: SQ Identifier (16 bits).
    ///
    /// The Submission Queue identifier that is associated with this completion.
    pub sq_id: u16,

    /// Dword 3, bits 0-15: Command Identifier (16 bits).
    ///
    /// The Command Identifier (CID) of the command that this completion is associated with.
    pub cid: u16,

    /// Dword 3, bits 16-31: Status Field (16 bits).
    pub status: u16,
}

impl NvmeCompletion {
    /// Checks if the completion indicates an error.
    ///
    /// Returns `true` if the Status Code is non-zero or DNR is set.
    pub(crate) fn has_error(&self) -> bool {
        (self.status & STATUS_ERROR_MASK) != 0
    }

    /// Gets the Status Code from the completion status field.
    ///
    /// Returns the 14-bit Status Code (bits 1-14).
    pub(crate) fn status_code(&self) -> u16 {
        (self.status & STATUS_ERROR_MASK) >> 1
    }
}

/// Submission Queue Entry.
#[repr(C)]
#[derive(Debug, Copy, Clone, Pod)]
pub(crate) struct NvmeCommand {
    /// Opcode.
    pub opcode: u8,
    /// Flags.
    pub flags: u8,
    /// Command ID.
    pub cid: u16,
    /// Namespace identifier.
    pub nsid: u32,
    /// Reserved.
    pub _rsvd: u64,
    /// Metadata pointer.
    pub mptr: u64,
    /// Data pointer.
    pub dptr: [u64; 2],
    /// Command dword 10.
    pub cdw10: u32,
    /// Command dword 11.
    pub cdw11: u32,
    /// Command dword 12.
    pub cdw12: u32,
    /// Command dword 13.
    pub cdw13: u32,
    /// Command dword 14.
    pub cdw14: u32,
    /// Command dword 15.
    pub cdw15: u32,
}

pub(crate) fn create_io_completion_queue(
    cid: u16,
    qid: u16,
    ptr: usize,
    size: u16,
    iv: Option<u16>,
) -> NvmeCommand {
    let cdw11 = if let Some(vector) = iv {
        ((vector as u32) << 16) | 0b11
    } else {
        0b1
    };

    NvmeCommand {
        opcode: AdminCommandSet::CreateIOCQ as u8,
        flags: 0,
        cid,
        nsid: 0,
        _rsvd: 0,
        mptr: 0,
        dptr: [ptr as u64, 0],
        cdw10: ((size as u32) << 16) | (qid as u32),
        cdw11,
        cdw12: 0,
        cdw13: 0,
        cdw14: 0,
        cdw15: 0,
    }
}

pub(crate) fn create_io_submission_queue(
    cid: u16,
    qid: u16,
    ptr: usize,
    size: u16,
    cqid: u16,
) -> NvmeCommand {
    NvmeCommand {
        opcode: AdminCommandSet::CreateIOSQ as u8,
        flags: 0,
        cid,
        nsid: 0,
        _rsvd: 0,
        mptr: 0,
        dptr: [ptr as u64, 0],
        cdw10: ((size as u32) << 16) | (qid as u32),
        cdw11: ((cqid as u32) << 16) | 1,
        cdw12: 0,
        cdw13: 0,
        cdw14: 0,
        cdw15: 0,
    }
}

pub(crate) fn identify_namespace(cid: u16, ptr: usize, nsid: u32) -> NvmeCommand {
    NvmeCommand {
        opcode: AdminCommandSet::IdentifyCommand as u8,
        flags: 0,
        cid,
        nsid,
        _rsvd: 0,
        mptr: 0,
        dptr: [ptr as u64, 0],
        cdw10: 0,
        cdw11: 0,
        cdw12: 0,
        cdw13: 0,
        cdw14: 0,
        cdw15: 0,
    }
}

pub(crate) fn identify_controller(cid: u16, ptr: usize) -> NvmeCommand {
    NvmeCommand {
        opcode: AdminCommandSet::IdentifyCommand as u8,
        flags: 0,
        cid,
        nsid: 0,
        _rsvd: 0,
        mptr: 0,
        dptr: [ptr as u64, 0],
        cdw10: 1,
        cdw11: 0,
        cdw12: 0,
        cdw13: 0,
        cdw14: 0,
        cdw15: 0,
    }
}

pub(crate) fn identify_namespace_list(cid: u16, ptr: usize, base: u32) -> NvmeCommand {
    NvmeCommand {
        opcode: AdminCommandSet::IdentifyCommand as u8,
        flags: 0,
        cid,
        nsid: base,
        _rsvd: 0,
        mptr: 0,
        dptr: [ptr as u64, 0],
        cdw10: 2,
        cdw11: 0,
        cdw12: 0,
        cdw13: 0,
        cdw14: 0,
        cdw15: 0,
    }
}

pub(crate) fn io_read(
    cid: u16,
    nsid: u32,
    lba: u64,
    blocks_1: u16,
    ptr0: u64,
    ptr1: u64,
) -> NvmeCommand {
    NvmeCommand {
        opcode: IoCommandSet::Read as u8,
        flags: 0 << NOT_FUSED_BITS,
        cid,
        nsid,
        _rsvd: 0,
        mptr: 0,
        dptr: [ptr0, ptr1],
        cdw10: lba as u32,
        cdw11: (lba >> 32) as u32,
        cdw12: blocks_1 as u32,
        cdw13: 0,
        cdw14: 0,
        cdw15: 0,
    }
}

pub(crate) fn io_write(
    cid: u16,
    nsid: u32,
    lba: u64,
    blocks_1: u16,
    ptr0: u64,
    ptr1: u64,
) -> NvmeCommand {
    NvmeCommand {
        opcode: IoCommandSet::Write as u8,
        flags: 0 << NOT_FUSED_BITS,
        cid,
        nsid,
        _rsvd: 0,
        mptr: 0,
        dptr: [ptr0, ptr1],
        cdw10: lba as u32,
        cdw11: (lba >> 32) as u32,
        cdw12: blocks_1 as u32,
        cdw13: 0,
        cdw14: 0,
        cdw15: 0,
    }
}

pub(crate) fn io_flush(cid: u16, nsid: u32) -> NvmeCommand {
    NvmeCommand {
        opcode: IoCommandSet::Flush as u8,
        flags: 0 << NOT_FUSED_BITS,
        cid,
        nsid,
        _rsvd: 0,
        mptr: 0,
        dptr: [0, 0],
        cdw10: 0,
        cdw11: 0,
        cdw12: 0,
        cdw13: 0,
        cdw14: 0,
        cdw15: 0,
    }
}
