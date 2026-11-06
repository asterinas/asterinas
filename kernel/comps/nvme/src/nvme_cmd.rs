// SPDX-License-Identifier: MPL-2.0

//! NVMe command builders.
//!
//! Refer to NVM Express Base Specification Revision 2.0:
//! - Section 5: Admin Command Set
//! - Section 6: NVM Command Set

use crate::nvme_spec::NvmeCommand;

/// Admin Command Set opcodes.
///
/// See NVMe Spec 2.0, Section 5 (Admin Command Set).
#[repr(u8)]
enum AdminCommandSet {
    /// Delete I/O Submission Queue command. See Section 5.7.
    DeleteIosq = 0x00,
    /// Create I/O Submission Queue command. See Section 5.5.
    CreateIosq = 0x01,
    /// Delete I/O Completion Queue command. See Section 5.6.
    DeleteIocq = 0x04,
    /// Create I/O Completion Queue command. See Section 5.4.
    CreateIocq = 0x05,
    /// Identify command. See Section 5.17.
    IdentifyCommand = 0x06,
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

/// Bit position for the FUSE (Fused Operation) field in the command flags byte.
///
/// The FUSE field (bits 6:7) indicates whether this command is part of a fused operation:
/// - 00b: Normal command (not part of a fused operation)
/// - 01b: First command of a fused operation
/// - 10b: Second command of a fused operation
/// - 11b: Reserved
const IO_CMD_NOT_FUSED_BITS: u8 = 6;

// Admin command builders for queue lifecycle.

/// Builds a Create I/O Completion Queue admin command. See Section 5.4.
pub(crate) fn create_io_completion_queue(
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

    NvmeCommand::from_raw_fields(
        AdminCommandSet::CreateIocq as u8,
        0,
        0,
        [ptr as u64, 0],
        [((size as u32) << 16) | (qid as u32), cdw11],
    )
}

/// Builds a Create I/O Submission Queue admin command. See Section 5.5.
pub(crate) fn create_io_submission_queue(
    qid: u16,
    ptr: usize,
    size: u16,
    cqid: u16,
) -> NvmeCommand {
    NvmeCommand::from_raw_fields(
        AdminCommandSet::CreateIosq as u8,
        0,
        0,
        [ptr as u64, 0],
        [
            ((size as u32) << 16) | (qid as u32),
            ((cqid as u32) << 16) | 1,
        ],
    )
}

/// Builds a Delete I/O Completion Queue admin command. See Section 5.6.
#[expect(dead_code)]
pub(crate) fn delete_io_completion_queue(qid: u16) -> NvmeCommand {
    NvmeCommand::from_raw_fields(
        AdminCommandSet::DeleteIocq as u8,
        0,
        0,
        [0, 0],
        [qid as u32],
    )
}

/// Builds a Delete I/O Submission Queue admin command. See Section 5.7.
#[expect(dead_code)]
pub(crate) fn delete_io_submission_queue(qid: u16) -> NvmeCommand {
    NvmeCommand::from_raw_fields(
        AdminCommandSet::DeleteIosq as u8,
        0,
        0,
        [0, 0],
        [qid as u32],
    )
}

// Admin command builders for identify operations.

/// Builds an Identify command for a single namespace (CNS 00h). See Section 5.17.
pub(crate) fn identify_namespace(ptr: usize, nsid: u32) -> NvmeCommand {
    NvmeCommand::from_raw_fields(
        AdminCommandSet::IdentifyCommand as u8,
        0,
        nsid,
        [ptr as u64, 0],
        [],
    )
}

/// Builds an Identify command for the controller (CNS 01h). See Section 5.17.
pub(crate) fn identify_controller(ptr: usize) -> NvmeCommand {
    NvmeCommand::from_raw_fields(
        AdminCommandSet::IdentifyCommand as u8,
        0,
        0,
        [ptr as u64, 0],
        [1],
    )
}

/// Builds an Identify command for the active namespace ID list (CNS 02h). See Section 5.17.
pub(crate) fn identify_namespace_list(ptr: usize, base: u32) -> NvmeCommand {
    NvmeCommand::from_raw_fields(
        AdminCommandSet::IdentifyCommand as u8,
        0,
        base,
        [ptr as u64, 0],
        [2],
    )
}

// I/O command builders.

/// Builds a Read command. See Section 7.
pub(crate) fn io_read(nsid: u32, lba: u64, nlb: u16, ptr0: u64, ptr1: u64) -> NvmeCommand {
    NvmeCommand::from_raw_fields(
        IoCommandSet::Read as u8,
        0 << IO_CMD_NOT_FUSED_BITS,
        nsid,
        [ptr0, ptr1],
        [
            lba as u32,
            (lba >> 32) as u32,
            // `nlb` is the Number of Logical Blocks field encoded as "block count minus one"
            nlb as u32,
        ],
    )
}

/// Builds a Write command. See Section 7.
pub(crate) fn io_write(nsid: u32, lba: u64, nlb: u16, ptr0: u64, ptr1: u64) -> NvmeCommand {
    NvmeCommand::from_raw_fields(
        IoCommandSet::Write as u8,
        0 << IO_CMD_NOT_FUSED_BITS,
        nsid,
        [ptr0, ptr1],
        [
            lba as u32,
            (lba >> 32) as u32,
            // `nlb` is the Number of Logical Blocks field encoded as "block count minus one"
            nlb as u32,
        ],
    )
}

/// Builds a Flush command. See Section 7.1.
pub(crate) fn io_flush(nsid: u32) -> NvmeCommand {
    NvmeCommand::from_raw_fields(
        IoCommandSet::Flush as u8,
        0 << IO_CMD_NOT_FUSED_BITS,
        nsid,
        [0, 0],
        [],
    )
}
