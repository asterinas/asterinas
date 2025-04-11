// SPDX-License-Identifier: MPL-2.0

use ostd::Pod;

const NOT_FUSED_BITS: u8 = 6;

#[repr(u8)]
enum AdminCommandSet {
    DeleteIOSQ = 0x00,
    CreateIOSQ = 0x01,
    DeleteIOCQ = 0x04,
    CreateIOCQ = 0x05,
    IdentifyCommand = 0x06,
}

#[repr(u8)]
enum IoCommandSet {
    Flush = 0x00,
    Write = 0x01,
    Read = 0x02,
}

/// The NVMe completion.
#[repr(C)]
#[derive(Debug, Copy, Clone, Pod)]
pub struct NVMeCompletion {
    /// The command-specific information
    pub command_specific: u32,
    /// Reserved
    pub rsvd: u32,
    /// The head pointer of corresponding submission queue
    pub sq_head: u16,
    /// The id of corresponding submission queue
    pub sq_id: u16,
    /// The command ID
    pub cid: u16,
    /// The status
    pub status: u16,
}

/// The NVMe command.
#[repr(C)]
#[derive(Debug, Copy, Clone, Pod)]
pub struct NVMeCommand {
    /// Opcode
    pub opcode: u8,
    /// Flags
    pub flags: u8,
    /// Command ID
    pub cid: u16,
    /// Namespace identifier
    pub nsid: u32,
    /// Reserved
    pub _rsvd: u64,
    /// Metadata pointer
    pub mptr: u64,
    /// Data pointer
    pub dptr: [u64; 2],
    /// Command dword 10
    pub cdw10: u32,
    /// Command dword 11
    pub cdw11: u32,
    /// Command dword 12
    pub cdw12: u32,
    /// Command dword 13
    pub cdw13: u32,
    /// Command dword 14
    pub cdw14: u32,
    /// Command dword 15
    pub cdw15: u32,
}

pub fn create_io_completion_queue(cid: u16, qid: u16, ptr: usize, size: u16) -> NVMeCommand {
    NVMeCommand {
        opcode: AdminCommandSet::CreateIOCQ as u8,
        flags: 0,
        cid,
        nsid: 0,
        _rsvd: 0,
        mptr: 0,
        dptr: [ptr as u64, 0],
        cdw10: ((size as u32) << 16) | (qid as u32),
        cdw11: 1,
        cdw12: 0,
        cdw13: 0,
        cdw14: 0,
        cdw15: 0,
    }
}

pub fn create_io_submission_queue(
    cid: u16,
    qid: u16,
    ptr: usize,
    size: u16,
    cqid: u16,
) -> NVMeCommand {
    NVMeCommand {
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

pub fn identify_namespace(cid: u16, ptr: usize, nsid: u32) -> NVMeCommand {
    NVMeCommand {
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

pub fn identify_controller(cid: u16, ptr: usize) -> NVMeCommand {
    NVMeCommand {
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

pub fn identify_namespace_list(cid: u16, ptr: usize, base: u32) -> NVMeCommand {
    NVMeCommand {
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

pub fn io_read(cid: u16, nsid: u32, lba: u64, blocks_1: u16, ptr0: u64, ptr1: u64) -> NVMeCommand {
    NVMeCommand {
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

pub fn io_write(cid: u16, nsid: u32, lba: u64, blocks_1: u16, ptr0: u64, ptr1: u64) -> NVMeCommand {
    NVMeCommand {
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

pub fn io_flush(cid: u16, nsid: u32) -> NVMeCommand {
    NVMeCommand {
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
