// SPDX-License-Identifier: MPL-2.0

//! `FUSE_RMDIR` removes a directory entry from a parent directory node.
//!
//! The request body contains only the null-terminated child name. Successful
//! replies do not carry a payload.

use ostd::mm::{Infallible, VmReader, VmWriter};

use super::util;
use crate::{FuseError, FuseOpcode, FuseOperation, FuseResult, ReplyExpectation};

pub struct RmdirOperation<'a> {
    name: &'a str,
}

impl<'a> RmdirOperation<'a> {
    pub fn new(name: &'a str) -> Self {
        Self { name }
    }
}

impl FuseOperation for RmdirOperation<'_> {
    type Output = ();

    fn opcode(&self) -> FuseOpcode {
        FuseOpcode::Rmdir
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
        ReplyExpectation::HeaderOnly
    }

    fn parse_reply(
        _payload_len: usize,
        _reader: &mut VmReader<'_, Infallible>,
    ) -> FuseResult<Self::Output> {
        Ok(())
    }
}
