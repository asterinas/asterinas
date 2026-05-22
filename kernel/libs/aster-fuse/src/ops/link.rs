// SPDX-License-Identifier: MPL-2.0

//! `FUSE_LINK` creates a hard link to an existing inode in a target directory.
//!
//! The request body contains [`LinkReq`] followed by the null-terminated new
//! name under the target parent directory node. The reply body contains
//! [`EntryReply`] for the linked inode.

use ostd::mm::{Infallible, VmReader, VmWriter};

use super::util;
use crate::{
    EntryReply, FuseError, FuseNodeId, FuseOpcode, FuseOperation, FuseResult, ReplyExpectation,
};

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub struct LinkReq {
    /// Existing inode that will receive the new hard link.
    oldnodeid: FuseNodeId,
}

impl LinkReq {
    pub const fn new(oldnodeid: FuseNodeId) -> Self {
        Self { oldnodeid }
    }
}

pub struct LinkOperation<'a> {
    link_req: LinkReq,
    new_name: &'a str,
}

impl<'a> LinkOperation<'a> {
    pub fn new(link_req: LinkReq, new_name: &'a str) -> Self {
        Self { link_req, new_name }
    }
}

impl FuseOperation for LinkOperation<'_> {
    type Output = EntryReply;

    fn opcode(&self) -> FuseOpcode {
        FuseOpcode::Link
    }

    fn body_len(&self) -> usize {
        util::name_body_len(size_of::<LinkReq>(), self.new_name)
    }

    fn write_body(&mut self, writer: &mut VmWriter<'_, Infallible>) -> FuseResult<()> {
        if writer.avail() < self.body_len() {
            return Err(FuseError::BufferTooSmall);
        }

        writer.write_val(&self.link_req).unwrap();
        writer.write(&mut VmReader::from(self.new_name.as_bytes()));
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
