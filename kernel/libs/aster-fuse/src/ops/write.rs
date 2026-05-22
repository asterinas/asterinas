// SPDX-License-Identifier: MPL-2.0

//! `FUSE_WRITE` writes bytes to an open file handle sitting at server-side.
//!
//! The request body contains [`WriteReq`] followed by the raw bytes to write.
//! The reply body contains [`WriteReply`], and the operation returns the number
//! of bytes accepted by the server.

use bitflags::bitflags;
use ostd::mm::{Infallible, VmReader, VmWriter};

use super::util::read_payload;
use crate::{FuseError, FuseFileHandle, FuseOpcode, FuseOperation, FuseResult, ReplyExpectation};

bitflags! {
    /// Flags for `FUSE_WRITE` requests.
    ///
    /// Reference: <https://elixir.bootlin.com/linux/v6.18/source/include/uapi/linux/fuse.h#L522-L531>
    #[repr(C)]
    #[derive(Pod)]
    pub struct WriteFlags: u32 {
        /// The write is serviced from the page cache (writeback mode).
        const WRITE_CACHE = 1 << 0;
        /// `lock_owner` is valid and should be used for lock-aware writes.
        const WRITE_LOCKOWNER = 1 << 1;
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub struct WriteReq {
    /// File handle to write to.
    fh: FuseFileHandle,
    /// File offset to start writing at.
    offset: u64,
    /// Number of bytes to write.
    size: u32,
    /// FUSE-specific write flags.
    write_flags: WriteFlags,
    /// Lock owner for lock-aware writes.
    lock_owner: u64,
    /// POSIX open flags associated with the handle.
    flags: u32,
    padding: u32,
}

impl WriteReq {
    pub const fn new(
        fh: FuseFileHandle,
        offset: u64,
        size: u32,
        flags: u32,
        write_flags: WriteFlags,
    ) -> Self {
        Self {
            fh,
            offset,
            size,
            write_flags,
            lock_owner: 0,
            flags,
            padding: 0,
        }
    }

    /// Returns the number of bytes to write.
    pub fn size(&self) -> u32 {
        self.size
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub struct WriteReply {
    /// Number of bytes written by the server.
    size: u32,
    padding: u32,
}

impl WriteReply {
    /// Returns the number of bytes written by the server.
    pub fn size(&self) -> usize {
        self.size as usize
    }
}

pub struct WriteOperation {
    write_req: WriteReq,
}

impl WriteOperation {
    pub fn new(write_req: WriteReq) -> Self {
        Self { write_req }
    }

    /// Returns the number of payload bytes to write.
    pub fn payload_size(&self) -> usize {
        self.write_req.size() as usize
    }
}

impl FuseOperation for WriteOperation {
    type Output = WriteReply;

    fn opcode(&self) -> FuseOpcode {
        FuseOpcode::Write
    }

    fn body_len(&self) -> usize {
        size_of::<WriteReq>()
    }

    fn write_body(&mut self, writer: &mut VmWriter<'_, Infallible>) -> FuseResult<()> {
        writer
            .write_val(&self.write_req)
            .map_err(|_| FuseError::BufferTooSmall)
    }

    fn reply_expectation(&self) -> ReplyExpectation {
        ReplyExpectation::payload(size_of::<WriteReply>())
    }

    fn parse_reply(
        payload_len: usize,
        reader: &mut VmReader<'_, Infallible>,
    ) -> FuseResult<Self::Output> {
        read_payload(payload_len, reader)
    }
}
