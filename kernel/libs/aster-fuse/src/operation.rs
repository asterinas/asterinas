// SPDX-License-Identifier: MPL-2.0

//! Defines the core trait for FUSE protocol operations.

use core::num::NonZeroUsize;

use int_to_c_enum::TryFromInt;
use ostd::mm::{Infallible, VmReader, VmWriter};

use crate::FuseResult;

/// A FUSE protocol operation with typed request and reply bodies.
///
/// Each implementer represents one request/reply pair defined by the FUSE protocol.
pub trait FuseOperation {
    /// Describes the successful reply produced by that operation.
    type Output;

    /// Returns the opcode identifying this operation's wire format.
    fn opcode(&self) -> FuseOpcode;

    /// Returns the request body length, excluding the `ReqHeader`.
    fn body_len(&self) -> usize {
        0
    }

    /// Writes the request body bytes into the transport buffer.
    ///
    /// The writer is positioned immediately after the `ReqHeader`.
    fn write_body(&mut self, _writer: &mut VmWriter<'_, Infallible>) -> FuseResult<()> {
        Ok(())
    }

    /// Returns the reply shape expected for this operation.
    fn reply_expectation(&self) -> ReplyExpectation;

    /// Parses the reply payload into [`Self::Output`].
    ///
    /// The reader is positioned at the start of the typed reply payload, after
    /// the `ReplyHeader`. Implementations must use `payload_len` to bound the
    /// bytes accepted from the server.
    fn parse_reply(
        payload_len: usize,
        reader: &mut VmReader<'_, Infallible>,
    ) -> FuseResult<Self::Output>;
}

/// A reply shape expected for a FUSE operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReplyExpectation {
    /// No reply is expected for a one-way request.
    None,
    /// A reply contains only a `ReplyHeader` and no payload bytes.
    HeaderOnly,
    /// A reply contains a `ReplyHeader` and a payload up to the given payload bytes.
    Payload(NonZeroUsize),
}

impl ReplyExpectation {
    /// Returns a payload-bearing reply expectation.
    ///
    /// A zero payload size is returned as [`Self::HeaderOnly`] so
    /// [`Self::Payload`] cannot encode the same wire shape.
    pub fn payload(payload_size: usize) -> Self {
        let Some(payload_size) = NonZeroUsize::new(payload_size) else {
            return Self::HeaderOnly;
        };
        Self::Payload(payload_size)
    }
}

#[repr(u32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, TryFromInt)]
pub enum FuseOpcode {
    Lookup = 1,
    Forget = 2,
    Getattr = 3,
    Setattr = 4,
    Readlink = 5,
    Symlink = 6,
    Mknod = 8,
    Mkdir = 9,
    Unlink = 10,
    Rmdir = 11,
    Rename = 12,
    Link = 13,
    Open = 14,
    Read = 15,
    Write = 16,
    Statfs = 17,
    Release = 18,
    Fsync = 20,
    Setxattr = 21,
    Getxattr = 22,
    Listxattr = 23,
    Removexattr = 24,
    Flush = 25,
    Init = 26,
    Opendir = 27,
    Readdir = 28,
    Releasedir = 29,
    Fsyncdir = 30,
    Getlk = 31,
    Setlk = 32,
    Setlkw = 33,
    Access = 34,
    Create = 35,
    Interrupt = 36,
    Bmap = 37,
    Destroy = 38,
    Ioctl = 39,
    Poll = 40,
    NotifyReply = 41,
    BatchForget = 42,
    Fallocate = 43,
    Readdirplus = 44,
    Rename2 = 45,
    Lseek = 46,
    CopyFileRange = 47,
    SetupMapping = 48,
    RemoveMapping = 49,
    SyncFs = 50,
    Tmpfile = 51,
}
