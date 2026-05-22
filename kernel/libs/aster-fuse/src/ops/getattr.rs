// SPDX-License-Identifier: MPL-2.0

//! `FUSE_GETATTR` reads attributes for an inode, optionally using an open file
//! handle carried in [`GetattrReq`].
//!
//! The reply body contains [`FuseAttrReply`], which includes the returned
//! attributes and their cache timeout.

use bitflags::bitflags;
use ostd::mm::{Infallible, VmReader, VmWriter};

use super::util::read_payload;
use crate::{
    Attr, EntryReply, FuseError, FuseFileHandle, FuseOpcode, FuseOperation, FuseResult,
    ReplyExpectation,
};

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub struct GetattrReq {
    getattr_flags: GetattrFlags,
    dummy: u32,
    /// File handle used when [`GetattrFlags::GETATTR_FH`] is set.
    fh: FuseFileHandle,
}

impl GetattrReq {
    pub const fn new(getattr_flags: GetattrFlags, fh: FuseFileHandle) -> Self {
        Self {
            getattr_flags,
            dummy: 0,
            fh,
        }
    }
}

bitflags! {
    /// Flags for `FUSE_GETATTR` requests.
    ///
    /// Reference: <https://elixir.bootlin.com/linux/v6.18/source/include/uapi/linux/fuse.h#L512-L515>
    #[repr(C)]
    #[derive(Pod)]
    pub struct GetattrFlags: u32 {
        /// The `fh` field is valid and identifies an open file.
        const GETATTR_FH = 1 << 0;
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub struct FuseAttrReply {
    /// Attribute-cache timeout in seconds.
    attr_valid: u64,
    /// Nanosecond component of [`FuseAttrReply::attr_valid`].
    attr_valid_nsec: u32,
    dummy: u32,
    attr: Attr,
}

impl FuseAttrReply {
    /// Creates a `FuseAttrReply` from the returned attributes and timeout.
    pub const fn new(attr_valid: u64, attr_valid_nsec: u32, attr: Attr) -> Self {
        Self {
            attr_valid,
            attr_valid_nsec,
            dummy: 0,
            attr,
        }
    }

    /// Returns the attribute-cache timeout in seconds.
    pub fn attr_valid(&self) -> u64 {
        self.attr_valid
    }

    /// Returns the nanosecond component of the attribute-cache timeout.
    pub fn attr_valid_nsec(&self) -> u32 {
        self.attr_valid_nsec
    }

    /// Returns the returned inode attributes.
    pub fn attr(&self) -> Attr {
        self.attr
    }
}

impl From<&EntryReply> for FuseAttrReply {
    fn from(entry_reply: &EntryReply) -> Self {
        Self::new(
            entry_reply.attr_valid(),
            entry_reply.attr_valid_nsec(),
            entry_reply.attr(),
        )
    }
}

pub struct GetattrOperation {
    getattr_req: GetattrReq,
}

impl GetattrOperation {
    pub fn new(getattr_req: GetattrReq) -> Self {
        Self { getattr_req }
    }
}

impl FuseOperation for GetattrOperation {
    type Output = FuseAttrReply;

    fn opcode(&self) -> FuseOpcode {
        FuseOpcode::Getattr
    }

    fn body_len(&self) -> usize {
        size_of::<GetattrReq>()
    }

    fn write_body(&mut self, writer: &mut VmWriter<'_, Infallible>) -> FuseResult<()> {
        writer
            .write_val(&self.getattr_req)
            .map_err(|_| FuseError::BufferTooSmall)
    }

    fn reply_expectation(&self) -> ReplyExpectation {
        ReplyExpectation::payload(size_of::<FuseAttrReply>())
    }

    fn parse_reply(
        payload_len: usize,
        reader: &mut VmReader<'_, Infallible>,
    ) -> FuseResult<Self::Output> {
        read_payload(payload_len, reader)
    }
}
