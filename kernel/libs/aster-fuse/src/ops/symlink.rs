// SPDX-License-Identifier: MPL-2.0

//! `FUSE_SYMLINK` creates a symbolic link under a parent directory node.
//!
//! The request body contains the null-terminated child name followed by the
//! null-terminated symbolic-link target. The reply body contains [`EntryReply`]
//! for the created symbolic-link inode.

use ostd::mm::{Infallible, VmReader, VmWriter};

use super::util;
use crate::{EntryReply, FuseError, FuseOpcode, FuseOperation, FuseResult, ReplyExpectation};

/// A request to create a symbolic link in a parent directory.
pub struct SymlinkOperation<'a> {
    name: &'a str,
    target: &'a str,
}

impl<'a> SymlinkOperation<'a> {
    /// Creates a symbolic-link request with the child name and target path.
    pub fn new(name: &'a str, target: &'a str) -> Self {
        Self { name, target }
    }
}

impl FuseOperation for SymlinkOperation<'_> {
    type Output = EntryReply;

    fn opcode(&self) -> FuseOpcode {
        FuseOpcode::Symlink
    }

    fn body_len(&self) -> usize {
        util::name_body_len(util::name_body_len(0, self.name), self.target)
    }

    fn write_body(&mut self, writer: &mut VmWriter<'_, Infallible>) -> FuseResult<()> {
        if writer.avail() < self.body_len() {
            return Err(FuseError::BufferTooSmall);
        }

        writer.write(&mut VmReader::from(self.name.as_bytes()));
        writer.write(&mut VmReader::from(util::NAME_TERMINATOR));
        writer.write(&mut VmReader::from(self.target.as_bytes()));
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
