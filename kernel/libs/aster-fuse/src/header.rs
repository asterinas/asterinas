// SPDX-License-Identifier: MPL-2.0

use crate::{FuseNodeId, FuseUnique};

/// The common header of a FUSE request.
///
/// Every request sent to a FUSE server starts with this header. The payload
/// that follows is determined by [`ReqHeader::opcode`].
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub struct ReqHeader {
    /// Total request length in bytes, including this header.
    len: u32,
    /// Operation code identifying the request payload format.
    opcode: u32,
    /// Request identifier copied into the matching [`ReplyHeader`].
    unique: FuseUnique,
    /// Target inode of the request.
    nodeid: FuseNodeId,
    /// User ID of the requesting process.
    uid: u32,
    /// Group ID of the requesting process.
    gid: u32,
    /// Process ID of the requesting process.
    pid: u32,
    /// Total length of extension headers that follow this header.
    total_extlen: u16,
    padding: u16,
}

impl ReqHeader {
    /// Creates a `ReqHeader` with the provided core fields.
    pub const fn new(len: u32, opcode: u32, unique: FuseUnique, nodeid: FuseNodeId) -> Self {
        Self {
            len,
            opcode,
            unique,
            nodeid,
            uid: 0,
            gid: 0,
            pid: 0,
            total_extlen: 0,
            padding: 0,
        }
    }

    /// Returns the total request length in bytes, including this header.
    pub fn len(&self) -> u32 {
        self.len
    }

    /// Returns the operation code identifying the request payload format.
    pub fn opcode(&self) -> u32 {
        self.opcode
    }

    /// Returns the request identifier copied into the matching reply.
    pub fn unique(&self) -> FuseUnique {
        self.unique
    }

    /// Returns the target inode of the request.
    pub fn nodeid(&self) -> FuseNodeId {
        self.nodeid
    }

    /// Returns whether the encoded request length is zero.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

/// The common header of a FUSE reply.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub struct ReplyHeader {
    /// Total reply length in bytes, including this header.
    len: u32,
    /// Operation result as `0` on success or a negated errno on failure.
    error: i32,
    /// Request identifier copied from the matching [`ReqHeader`].
    unique: FuseUnique,
}

impl ReplyHeader {
    pub const fn new(len: u32, error: i32, unique: FuseUnique) -> Self {
        Self { len, error, unique }
    }

    /// Returns an empty [`ReplyHeader`].
    pub const fn empty() -> Self {
        Self::new(0, 0, FuseUnique::new(0))
    }

    /// Returns the total reply length in bytes, including this header.
    pub fn len(&self) -> u32 {
        self.len
    }

    /// Returns the operation result as `0` on success or a negated errno on failure.
    pub fn error(&self) -> i32 {
        self.error
    }

    /// Returns the request identifier copied from the matching request.
    pub fn unique(&self) -> FuseUnique {
        self.unique
    }

    /// Returns whether the encoded reply length is zero.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}
