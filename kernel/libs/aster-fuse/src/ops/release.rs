// SPDX-License-Identifier: MPL-2.0

//! `FUSE_RELEASE` releases an open file handle, and `FUSE_RELEASEDIR` releases
//! an open directory handle.
//!
//! Both request bodies contain [`ReleaseReq`] for the handle named by the target
//! inode and [`ReleaseKind`] selects the opcode. Successful replies do not
//! carry a payload.

use bitflags::bitflags;
use ostd::mm::{Infallible, VmReader, VmWriter};

use crate::{FuseError, FuseFileHandle, FuseOpcode, FuseOperation, FuseResult, ReplyExpectation};

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub struct ReleaseReq {
    /// File handle to release.
    fh: FuseFileHandle,
    /// POSIX open flags associated with the handle.
    flags: u32,
    /// FUSE-specific release flags.
    release_flags: ReleaseFlags,
    /// Lock owner associated with the handle.
    lock_owner: u64,
}

impl ReleaseReq {
    pub const fn new(fh: FuseFileHandle, flags: u32, release_flags: ReleaseFlags) -> Self {
        Self {
            fh,
            flags,
            release_flags,
            lock_owner: 0,
        }
    }
}

bitflags! {
    /// Flags for `FUSE_RELEASE` and `FUSE_RELEASEDIR` requests.
    ///
    /// Reference: <https://elixir.bootlin.com/linux/v6.18/source/include/uapi/linux/fuse.h#L506-L510>
    #[repr(C)]
    #[derive(Pod)]
    pub struct ReleaseFlags: u32 {
        /// The contents of the file should be flushed to disk.
        const RELEASE_FLUSH = 1 << 0;
        /// Unlocks BSD flock locks held by the process on release.
        const RELEASE_FLOCK_UNLOCK = 1 << 1;
    }
}

pub struct ReleaseOperation {
    release_req: ReleaseReq,
    release_options: ReleaseOptions,
}

impl ReleaseOperation {
    pub fn new(fh: FuseFileHandle, flags: u32, release_options: ReleaseOptions) -> Self {
        Self {
            release_req: ReleaseReq::new(fh, flags, release_options.flags),
            release_options,
        }
    }
}

impl FuseOperation for ReleaseOperation {
    type Output = ();

    fn opcode(&self) -> FuseOpcode {
        self.release_options.kind.opcode()
    }

    fn body_len(&self) -> usize {
        size_of::<ReleaseReq>()
    }

    fn write_body(&mut self, writer: &mut VmWriter<'_, Infallible>) -> FuseResult<()> {
        writer
            .write_val(&self.release_req)
            .map_err(|_| FuseError::BufferTooSmall)
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

/// Options that determine how an open handle is released.
#[derive(Clone, Copy, Debug)]
pub struct ReleaseOptions {
    kind: ReleaseKind,
    flags: ReleaseFlags,
}

impl ReleaseOptions {
    pub const fn new(kind: ReleaseKind, flags: ReleaseFlags) -> Self {
        Self { kind, flags }
    }

    pub const fn kind(self) -> ReleaseKind {
        self.kind
    }

    pub const fn flags(self) -> ReleaseFlags {
        self.flags
    }
}

/// Selects between `FUSE_RELEASE` (for files) and `FUSE_RELEASEDIR` (for
/// directories). The protocol uses distinct opcodes for each.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReleaseKind {
    /// Releases a regular file handle with `FUSE_RELEASE`.
    File,
    /// Releases a directory handle with `FUSE_RELEASEDIR`.
    Directory,
}

impl ReleaseKind {
    fn opcode(self) -> FuseOpcode {
        match self {
            Self::File => FuseOpcode::Release,
            Self::Directory => FuseOpcode::Releasedir,
        }
    }
}
