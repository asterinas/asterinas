// SPDX-License-Identifier: MPL-2.0

//! `FUSE_STATFS` reads filesystem capacity and inode statistics.

use ostd::mm::{Infallible, VmReader};

use super::util::read_payload;
use crate::{FuseOpcode, FuseOperation, FuseResult, ReplyExpectation};

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub struct Kstatfs {
    blocks: u64,
    bfree: u64,
    bavail: u64,
    files: u64,
    ffree: u64,
    bsize: u32,
    namelen: u32,
    frsize: u32,
    padding: u32,
    spare: [u32; 6],
}

impl Kstatfs {
    /// Returns the total number of data blocks.
    pub fn blocks(&self) -> u64 {
        self.blocks
    }

    /// Returns the number of free blocks.
    pub fn bfree(&self) -> u64 {
        self.bfree
    }

    /// Returns the number of free blocks available to unprivileged users.
    pub fn bavail(&self) -> u64 {
        self.bavail
    }

    /// Returns the total number of inodes.
    pub fn files(&self) -> u64 {
        self.files
    }

    /// Returns the number of free inodes.
    pub fn ffree(&self) -> u64 {
        self.ffree
    }

    /// Returns the filesystem block size.
    pub fn bsize(&self) -> u32 {
        self.bsize
    }

    /// Returns the maximum filename length.
    pub fn namelen(&self) -> u32 {
        self.namelen
    }

    /// Returns the fragment size.
    pub fn frsize(&self) -> u32 {
        self.frsize
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub struct StatfsReply {
    st: Kstatfs,
}

impl StatfsReply {
    /// Returns the filesystem statistics returned by the server.
    pub fn st(&self) -> Kstatfs {
        self.st
    }
}

pub struct StatfsOperation;

impl FuseOperation for StatfsOperation {
    type Output = StatfsReply;

    fn opcode(&self) -> FuseOpcode {
        FuseOpcode::Statfs
    }

    fn reply_expectation(&self) -> ReplyExpectation {
        ReplyExpectation::payload(size_of::<StatfsReply>())
    }

    fn parse_reply(
        payload_len: usize,
        reader: &mut VmReader<'_, Infallible>,
    ) -> FuseResult<Self::Output> {
        read_payload(payload_len, reader)
    }
}
