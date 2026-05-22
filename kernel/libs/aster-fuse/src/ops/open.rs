// SPDX-License-Identifier: MPL-2.0

//! `FUSE_OPEN` opens a non-directory inode, and `FUSE_OPENDIR` opens a
//! directory inode.
//!
//! Both request bodies contain [`OpenReq`] with the requested open flags, and
//! both operations return the [`OpenReply`] reply.

use bitflags::bitflags;
use ostd::mm::{Infallible, VmReader, VmWriter};

use super::util::read_payload;
use crate::{FuseError, FuseFileHandle, FuseOpcode, FuseOperation, FuseResult, ReplyExpectation};

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub struct OpenReq {
    /// POSIX open flags.
    flags: u32,
    /// FUSE-specific open flags.
    open_flags: FuseOpenFlags,
}

impl OpenReq {
    pub const fn new(flags: u32) -> Self {
        Self {
            flags,
            open_flags: FuseOpenFlags::empty(),
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub struct OpenReply {
    /// File handle assigned by the server.
    fh: FuseFileHandle,
    /// FUSE-specific open result flags.
    open_flags: FuseOpenFlags,
    padding: u32,
}

impl OpenReply {
    /// Returns the file handle assigned by the server.
    pub fn fh(&self) -> FuseFileHandle {
        self.fh
    }

    /// Returns the FUSE-specific open result flags.
    pub fn open_flags(&self) -> FuseOpenFlags {
        self.open_flags
    }
}

bitflags! {
    /// FUSE-specific flags returned by open replies.
    ///
    /// Reference: <https://elixir.bootlin.com/linux/v6.18/source/include/uapi/linux/fuse.h#L387-L394>
    #[repr(C)]
    #[derive(Pod)]
    pub struct FuseOpenFlags: u32 {
        /// Bypasses the page cache for this open file.
        const FOPEN_DIRECT_IO = 1 << 0;
        /// Keeps cached file data valid when this file is opened.
        const FOPEN_KEEP_CACHE = 1 << 1;
        /// Marks this open file as non-seekable.
        const FOPEN_NONSEEKABLE = 1 << 2;
        /// Allows caching directory entries for this open directory.
        const FOPEN_CACHE_DIR = 1 << 3;
        /// Marks this open file as stream-like, with no file position.
        const FOPEN_STREAM = 1 << 4;
        /// Skips flushing cached data on close unless writeback caching is enabled.
        const FOPEN_NOFLUSH = 1 << 5;
        /// Allows concurrent direct writes on the same inode.
        const FOPEN_PARALLEL_DIRECT_WRITES = 1 << 6;
        /// Enables passthrough read and write I/O for this open file.
        const FOPEN_PASSTHROUGH = 1 << 7;
    }
}

pub struct OpenOperation {
    open_req: OpenReq,
}

impl OpenOperation {
    pub fn new(open_req: OpenReq) -> Self {
        Self { open_req }
    }
}

impl FuseOperation for OpenOperation {
    type Output = OpenReply;

    fn opcode(&self) -> FuseOpcode {
        FuseOpcode::Open
    }

    fn body_len(&self) -> usize {
        size_of::<OpenReq>()
    }

    fn write_body(&mut self, writer: &mut VmWriter<'_, Infallible>) -> FuseResult<()> {
        writer
            .write_val(&self.open_req)
            .map_err(|_| FuseError::BufferTooSmall)
    }

    fn reply_expectation(&self) -> ReplyExpectation {
        ReplyExpectation::payload(size_of::<OpenReply>())
    }

    fn parse_reply(
        payload_len: usize,
        reader: &mut VmReader<'_, Infallible>,
    ) -> FuseResult<Self::Output> {
        read_payload(payload_len, reader)
    }
}

pub struct OpendirOperation {
    open_req: OpenReq,
}

impl OpendirOperation {
    pub fn new(open_req: OpenReq) -> Self {
        Self { open_req }
    }
}

impl FuseOperation for OpendirOperation {
    type Output = OpenReply;

    fn opcode(&self) -> FuseOpcode {
        FuseOpcode::Opendir
    }

    fn body_len(&self) -> usize {
        size_of::<OpenReq>()
    }

    fn write_body(&mut self, writer: &mut VmWriter<'_, Infallible>) -> FuseResult<()> {
        writer.write_val(&self.open_req).unwrap();

        Ok(())
    }

    fn reply_expectation(&self) -> ReplyExpectation {
        ReplyExpectation::payload(size_of::<OpenReply>())
    }

    fn parse_reply(
        payload_len: usize,
        reader: &mut VmReader<'_, Infallible>,
    ) -> FuseResult<Self::Output> {
        read_payload(payload_len, reader)
    }
}
