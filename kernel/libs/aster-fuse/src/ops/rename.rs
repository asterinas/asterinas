// SPDX-License-Identifier: MPL-2.0

//! `FUSE_RENAME` renames a directory entry.
//!
//! The request body contains [`RenameReq`] followed by the null-terminated old
//! name and the null-terminated new name. Successful replies do not carry a
//! payload.

use ostd::mm::{Infallible, VmReader, VmWriter};

use super::util;
use crate::{FuseError, FuseNodeId, FuseOpcode, FuseOperation, FuseResult, ReplyExpectation};

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub struct RenameReq {
    /// Parent directory node that will receive the renamed entry.
    newdir: FuseNodeId,
}

impl RenameReq {
    pub const fn new(newdir: FuseNodeId) -> Self {
        Self { newdir }
    }
}

pub struct RenameOperation<'a> {
    rename_req: RenameReq,
    old_name: &'a str,
    new_name: &'a str,
}

impl<'a> RenameOperation<'a> {
    pub fn new(rename_req: RenameReq, old_name: &'a str, new_name: &'a str) -> Self {
        Self {
            rename_req,
            old_name,
            new_name,
        }
    }
}

impl FuseOperation for RenameOperation<'_> {
    type Output = ();

    fn opcode(&self) -> FuseOpcode {
        FuseOpcode::Rename
    }

    fn body_len(&self) -> usize {
        util::name_body_len(
            util::name_body_len(size_of::<RenameReq>(), self.old_name),
            self.new_name,
        )
    }

    fn write_body(&mut self, writer: &mut VmWriter<'_, Infallible>) -> FuseResult<()> {
        if writer.avail() < self.body_len() {
            return Err(FuseError::BufferTooSmall);
        }

        writer.write_val(&self.rename_req).unwrap();
        writer.write(&mut VmReader::from(self.old_name.as_bytes()));
        writer.write(&mut VmReader::from(util::NAME_TERMINATOR));
        writer.write(&mut VmReader::from(self.new_name.as_bytes()));
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
