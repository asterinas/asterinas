// SPDX-License-Identifier: MPL-2.0

//! `FUSE_LOOKUP` resolves a child name under a parent directory node.
//!
//! The request body contains only the null-terminated child name. The reply
//! body contains [`EntryReply`] for the resolved inode.

use ostd::mm::{Infallible, VmReader, VmWriter};

use super::util;
use crate::{EntryReply, FuseError, FuseOpcode, FuseOperation, FuseResult, ReplyExpectation};

pub struct LookupOperation<'a> {
    name: &'a str,
}

impl<'a> LookupOperation<'a> {
    pub fn new(name: &'a str) -> Self {
        Self { name }
    }
}

impl FuseOperation for LookupOperation<'_> {
    type Output = EntryReply;

    fn opcode(&self) -> FuseOpcode {
        FuseOpcode::Lookup
    }

    fn body_len(&self) -> usize {
        util::name_body_len(0, self.name)
    }

    fn write_body(&mut self, writer: &mut VmWriter<'_, Infallible>) -> FuseResult<()> {
        if writer.avail() < self.body_len() {
            return Err(FuseError::BufferTooSmall);
        }

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
