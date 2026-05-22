// SPDX-License-Identifier: MPL-2.0

//! `FUSE_INIT` negotiates the protocol version and capabilities for a FUSE
//! connection.
//!
//! The request body contains [`InitReq`] with the client-supported version,
//! readahead size, and feature flags. The reply body contains [`InitReply`] with
//! the server-selected version, limits, and negotiated flags.

use bitflags::bitflags;
use ostd::mm::{Infallible, VmReader, VmWriter};
use ostd_pod::{FromZeros, IntoBytes};

use super::util::read_bytes;
use crate::{FuseError, FuseOpcode, FuseOperation, FuseResult, ReplyExpectation};

/// The minimum valid `FUSE_INIT` reply payload size for legacy servers.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.18/source/include/uapi/linux/fuse.h#L911>
const FUSE_COMPAT_INIT_OUT_SIZE: usize = 8;

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub struct InitReq {
    /// Major version of the FUSE protocol supported by the client.
    major: u32,
    /// Minor version of the FUSE protocol supported by the client.
    minor: u32,
    /// Maximum readahead size requested by the client.
    max_readahead: u32,
    /// Lower 32 bits of supported client capabilities.
    flags: FuseInitFlags,
    /// The following fields are extensions.
    ///
    /// Upper 32 bits of supported client capabilities.
    flags2: FuseInitFlags2,
    unused: [u32; 11],
}

impl InitReq {
    pub const fn new(
        major: u32,
        minor: u32,
        max_readahead: u32,
        flags: FuseInitFlags,
        flags2: FuseInitFlags2,
    ) -> Self {
        Self {
            major,
            minor,
            max_readahead,
            flags,
            flags2,
            unused: [0; 11],
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub struct InitReply {
    /// Major version of the FUSE protocol selected by the server.
    major: u32,
    /// Minor version of the FUSE protocol selected by the server.
    minor: u32,
    /// Maximum readahead size accepted by the server.
    max_readahead: u32,
    /// Lower 32 bits of negotiated capabilities.
    flags: FuseInitFlags,
    /// Maximum number of background requests.
    max_background: u16,
    /// Background request threshold for congestion.
    congestion_threshold: u16,
    /// Maximum write size accepted by the server.
    max_write: u32,
    /// Timestamp granularity in nanoseconds.
    time_gran: u32,
    /// Maximum number of pages in a request.
    max_pages: u16,
    /// Mapping alignment requirement as a power-of-two page count.
    map_alignment: u16,
    /// Upper 32 bits of negotiated capabilities.
    flags2: FuseInitFlags2,
    /// Maximum stack depth for passthrough operations.
    max_stack_depth: u32,
    /// Request timeout in seconds.
    request_timeout: u16,
    unused: [u16; 11],
}

impl InitReply {
    /// Returns the FUSE protocol major version selected by the server.
    pub fn major(&self) -> u32 {
        self.major
    }

    /// Returns the FUSE protocol minor version selected by the server.
    pub fn minor(&self) -> u32 {
        self.minor
    }

    /// Returns the maximum readahead size accepted by the server.
    pub fn max_readahead(&self) -> u32 {
        self.max_readahead
    }

    /// Returns the lower 32 bits of negotiated capabilities.
    pub fn flags(&self) -> FuseInitFlags {
        self.flags
    }

    /// Returns the maximum write size accepted by the server.
    pub fn max_write(&self) -> u32 {
        self.max_write
    }

    /// Returns the timestamp granularity in nanoseconds.
    pub fn time_gran(&self) -> u32 {
        self.time_gran
    }

    /// Returns the maximum number of pages in a request.
    pub fn max_pages(&self) -> u16 {
        self.max_pages
    }

    /// Returns the mapping alignment requirement as a power-of-two page count.
    pub fn map_alignment(&self) -> u16 {
        self.map_alignment
    }

    /// Returns the upper 32 bits of negotiated capabilities.
    pub fn flags2(&self) -> FuseInitFlags2 {
        self.flags2
    }
}

bitflags! {
    /// FUSE capability and feature flags exchanged in `FUSE_INIT`.
    ///
    /// The client sends its supported set in [`InitReq::new`]; the server
    /// responds with the subset it also supports in [`InitReply::flags`].
    #[repr(C)]
    #[derive(Pod)]
    pub struct FuseInitFlags: u32 {
        /// Supports asynchronous reads.
        const ASYNC_READ          = 1 << 0;
        /// Supports POSIX byte-range locks.
        const POSIX_LOCKS         = 1 << 1;
        /// Uses file-handle based operations.
        const FILE_OPS            = 1 << 2;
        /// Supports atomic `O_TRUNC` handling during open.
        const ATOMIC_O_TRUNC      = 1 << 3;
        /// Supports stable inode numbers for export.
        const EXPORT_SUPPORT      = 1 << 4;
        /// Supports writes larger than 4 KiB.
        const BIG_WRITES          = 1 << 5;
        /// Preserves mode bits instead of applying the process umask.
        const DONT_MASK           = 1 << 6;
        /// Supports splice-based writes.
        const SPLICE_WRITE        = 1 << 7;
        /// Supports splice move optimization.
        const SPLICE_MOVE         = 1 << 8;
        /// Supports splice-based reads.
        const SPLICE_READ         = 1 << 9;
        /// Supports BSD-style flock locks.
        const FLOCK_LOCKS         = 1 << 10;
        /// Supports ioctl requests on directories.
        const HAS_IOCTL_DIR       = 1 << 11;
        /// Invalidates cached file data automatically on attribute changes.
        const AUTO_INVAL_DATA     = 1 << 12;
        /// Supports `FUSE_READDIRPLUS`.
        const DO_READDIRPLUS      = 1 << 13;
        /// Allows the server to choose when to use `FUSE_READDIRPLUS`.
        const READDIRPLUS_AUTO    = 1 << 14;
        /// Supports asynchronous direct I/O.
        const ASYNC_DIO           = 1 << 15;
        /// Supports writeback caching.
        const WRITEBACK_CACHE     = 1 << 16;
        /// Allows `ENOSYS` from `FUSE_OPEN` to mean open is unsupported.
        const NO_OPEN_SUPPORT     = 1 << 17;
        /// Supports parallel directory operations.
        const PARALLEL_DIROPS     = 1 << 18;
        /// Lets the server clear privilege bits after writes and truncates.
        const HANDLE_KILLPRIV     = 1 << 19;
        /// Supports POSIX ACLs.
        const POSIX_ACL           = 1 << 20;
        /// Supports returning an error from abort handling.
        const ABORT_ERROR         = 1 << 21;
        /// Supports the `max_pages` field in [`InitReply`].
        const MAX_PAGES           = 1 << 22;
        /// Supports caching symbolic-link targets.
        const CACHE_SYMLINKS      = 1 << 23;
        /// Allows `ENOSYS` from `FUSE_OPENDIR` to mean opendir is unsupported.
        const NO_OPENDIR_SUPPORT  = 1 << 24;
        /// Supports explicit file-data invalidation.
        const EXPLICIT_INVAL_DATA = 1 << 25;
        /// Supports the `map_alignment` field in [`InitReply`].
        const MAP_ALIGNMENT       = 1 << 26;
        /// Supports submounts.
        const SUBMOUNTS           = 1 << 27;
        /// Supports the version-2 privilege-bit clearing protocol.
        const HANDLE_KILLPRIV_V2  = 1 << 28;
        /// Supports extended setxattr requests.
        const SETXATTR_EXT        = 1 << 29;
        /// Supports extended `FUSE_INIT` fields.
        const INIT_EXT            = 1 << 30;
        /// Reserved by the FUSE protocol.
        const INIT_RESERVED       = 1 << 31;
    }
}

bitflags! {
    /// Upper FUSE capability and feature flags exchanged in `FUSE_INIT`.
    ///
    /// These flags encode bits 32 through 63 of the protocol capability set
    /// after shifting them down into the `flags2` field.
    #[repr(C)]
    #[derive(Pod)]
    pub struct FuseInitFlags2: u32 {
        /// Supports security context information.
        const SECURITY_CTX             = 1 << 0;
        /// Supports inode DAX information.
        const HAS_INODE_DAX           = 1 << 1;
        /// Supports supplementary groups during create.
        const CREATE_SUPP_GROUP       = 1 << 2;
        /// Supports expire-only invalidation.
        const HAS_EXPIRE_ONLY         = 1 << 3;
        /// Allows memory maps on direct-I/O files.
        const DIRECT_IO_ALLOW_MMAP    = 1 << 4;
        /// Supports passthrough operations.
        const PASSTHROUGH             = 1 << 5;
        /// Supports disabling export support.
        const NO_EXPORT_SUPPORT       = 1 << 6;
        /// Supports resending requests.
        const HAS_RESEND              = 1 << 7;
        /// Allows ID-mapped mounts.
        const ALLOW_IDMAP             = 1 << 8;
        /// Supports FUSE over io_uring.
        const OVER_IO_URING           = 1 << 9;
        /// Supports request timeouts.
        const REQUEST_TIMEOUT         = 1 << 10;
    }
}

pub struct InitOperation {
    init_req: InitReq,
}

impl InitOperation {
    pub fn new(init_req: InitReq) -> Self {
        Self { init_req }
    }
}

impl FuseOperation for InitOperation {
    type Output = InitReply;

    fn opcode(&self) -> FuseOpcode {
        FuseOpcode::Init
    }

    fn body_len(&self) -> usize {
        size_of::<InitReq>()
    }

    fn write_body(&mut self, writer: &mut VmWriter<'_, Infallible>) -> FuseResult<()> {
        writer
            .write_val(&self.init_req)
            .map_err(|_| FuseError::BufferTooSmall)
    }

    fn reply_expectation(&self) -> ReplyExpectation {
        ReplyExpectation::payload(size_of::<InitReply>())
    }

    fn parse_reply(
        payload_len: usize,
        reader: &mut VmReader<'_, Infallible>,
    ) -> FuseResult<Self::Output> {
        if payload_len < FUSE_COMPAT_INIT_OUT_SIZE {
            return Err(FuseError::MalformedResponse);
        }

        let mut init_reply = InitReply::new_zeroed();
        let read_len = core::cmp::min(payload_len, size_of::<InitReply>());
        read_bytes(reader, &mut init_reply.as_mut_bytes()[..read_len])?;

        Ok(init_reply)
    }
}
