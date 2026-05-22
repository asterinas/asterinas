// SPDX-License-Identifier: MPL-2.0

//! `FUSE_MKDIR` creates a directory under a parent directory node.
//!
//! The request body contains [`MkdirReq`] followed by the null-terminated child
//! name. The reply body contains [`EntryReply`] for the created directory inode.

use ostd::mm::{Infallible, VmReader, VmWriter};

use super::util;
use crate::{EntryReply, FuseError, FuseOpcode, FuseOperation, FuseResult, ReplyExpectation};

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub struct MkdirReq {
    /// File type and permission bits for the new directory.
    mode: u32,
    /// Process umask of the requesting client, applied by the server when creating the inode.
    umask: u32,
}

impl MkdirReq {
    pub const fn new(mode: u32) -> Self {
        Self { mode, umask: 0 }
    }
}

pub struct MkdirOperation<'a> {
    mkdir_req: MkdirReq,
    name: &'a str,
}

impl<'a> MkdirOperation<'a> {
    pub fn new(mkdir_req: MkdirReq, name: &'a str) -> Self {
        Self { mkdir_req, name }
    }
}

impl FuseOperation for MkdirOperation<'_> {
    type Output = EntryReply;

    fn opcode(&self) -> FuseOpcode {
        FuseOpcode::Mkdir
    }

    fn body_len(&self) -> usize {
        util::name_body_len(size_of::<MkdirReq>(), self.name)
    }

    fn write_body(&mut self, writer: &mut VmWriter<'_, Infallible>) -> FuseResult<()> {
        if writer.avail() < self.body_len() {
            return Err(FuseError::BufferTooSmall);
        }

        writer.write_val(&self.mkdir_req).unwrap();
        writer.write(&mut VmReader::from(self.name.as_bytes()));
        writer.write(&mut VmReader::from(util::NAME_TERMINATOR));

        Ok(())
    }

    fn reply_expectation(&self) -> ReplyExpectation {
        ReplyExpectation::payload(size_of::<EntryReply>())
    }

    fn parse_reply(
        payload_len: usize,
        reader: &mut VmReader<'_, Infallible>,
    ) -> FuseResult<Self::Output> {
        util::read_payload(payload_len, reader)
    }
}
