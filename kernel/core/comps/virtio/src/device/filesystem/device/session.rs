// SPDX-License-Identifier: MPL-2.0

//! FUSE session connection for `virtiofs`.
//!
//! [`FuseSession`] is mount-scoped FUSE session. It performs
//! `FUSE_INIT` negotiation, and exposes typed request helpers.

use alloc::{string::String, sync::Arc, vec::Vec};
use core::sync::atomic::{AtomicU64, Ordering};

use aster_fuse::{
    FUSE_KERNEL_MINOR_VERSION, FUSE_KERNEL_VERSION, FUSE_ROOT_ID, FuseCompleteFn, FuseDirEntry,
    FuseError, FuseFileHandle, FuseNodeId, FuseOperation, MIN_MAX_WRITE,
    ops::{
        forget::{ForgetOperation, ForgetReq},
        init::{FuseInitFlags, FuseInitFlags2, InitOperation, InitReq},
        read::{ReadOperation, ReadReq},
        readdir::ReaddirOperation,
        readlink::{MAX_READLINK_LEN, ReadlinkOperation},
        release::{ReleaseOperation, ReleaseOptions},
        write::{WriteOperation, WriteReq},
    },
};
use ostd::{info, mm::io::util::HasVmReaderWriter};

use super::{super::DEVICE_NAME, FileSystemDevice, FuseWaiter};
use crate::device::filesystem::pool::{FuseDataBuf, FuseReplyBuf, FuseRequestBuf};

/// A mount-scoped FUSE session.
///
/// One `FuseSession` corresponds to one `mount(2)` call. It holds the
/// negotiated FUSE protocol state and forwards typed requests to the
/// underlying [`FileSystemDevice`].
pub struct FuseSession {
    /// The transport used to submit FUSE requests for this session.
    device: Arc<FileSystemDevice>,
    /// Attribute cache version shared by all inodes in this session.
    attr_version: AtomicU64,
    /// The maximum write size accepted by the server.
    max_write: u32,
    /// The feature flags selected by `FUSE_INIT`.
    //
    // TODO: Apply negotiated `FUSE_INIT` flags to conduct virtio-fs behavior.
    negotiated_flags: FuseInitFlags,
}

impl FuseSession {
    /// Creates a new FUSE session by performing `FUSE_INIT` negotiation with
    /// the server.
    pub fn new(device: Arc<FileSystemDevice>) -> Result<Arc<Self>, FuseError> {
        let requested_flags = Self::init_flags();
        let mut operation = InitOperation::new(InitReq::new(
            FUSE_KERNEL_VERSION,
            FUSE_KERNEL_MINOR_VERSION,
            0,
            requested_flags,
            FuseInitFlags2::empty(),
        ));
        let waiter = device.submit_fuse_op(FUSE_ROOT_ID, &mut operation, None, None)?;
        let payload_len = waiter.wait().payload_len()?;
        let init_reply = waiter.parse_reply::<InitOperation>(payload_len)?;

        let max_write = init_reply.max_write().max(MIN_MAX_WRITE);
        let session = Arc::new(Self {
            device,
            attr_version: AtomicU64::new(1),
            max_write,
            negotiated_flags: init_reply.flags(),
        });

        info!(
            "{} FUSE session started: protocol {}.{} -> {}.{}, \
             req_flags=0x{:x}, rsp_flags=0x{:x}, flags2=0x{:x}, \
             max_write={}, max_readahead={}, time_gran={}, max_pages={}, map_alignment={}",
            DEVICE_NAME,
            FUSE_KERNEL_VERSION,
            FUSE_KERNEL_MINOR_VERSION,
            init_reply.major(),
            init_reply.minor(),
            requested_flags.bits(),
            session.negotiated_flags.bits(),
            init_reply.flags2().bits(),
            session.max_write,
            init_reply.max_readahead(),
            init_reply.time_gran(),
            init_reply.max_pages(),
            init_reply.map_alignment(),
        );

        Ok(session)
    }

    /// Sends one FUSE operation and waits for the typed reply.
    ///
    /// # Locking
    ///
    /// This method may sleep after submission. Callers but must
    /// not hold spinlocks, or IRQ-disabled guards across this call.
    pub fn do_fuse_op<Op: FuseOperation>(
        &self,
        nodeid: FuseNodeId,
        mut operation: Op,
    ) -> Result<Op::Output, FuseError> {
        let waiter = self
            .device
            .submit_fuse_op(nodeid, &mut operation, None, None)?;
        let payload_len = waiter.wait().payload_len()?;
        waiter.parse_reply::<Op>(payload_len)
    }

    /// Returns the current attribute version for a request snapshot.
    pub fn snapshot_attr_version(&self) -> AttrVersion {
        // The snapshot is only a comparable timestamp for stale-reply detection.
        // It does not publish or consume inode attributes, so no acquire/release
        // ordering is needed.
        AttrVersion::new(self.attr_version.load(Ordering::Relaxed))
    }

    /// Commits a new attribute version and returns it.
    pub fn bump_attr_version(&self) -> AttrVersion {
        // FIXME: Handle potential overflow of the version counter, which may cause stale.
        AttrVersion::new(self.attr_version.fetch_add(1, Ordering::Relaxed) + 1)
    }

    /// Allocates a buffer for `FUSE_READ` data.
    pub fn alloc_read_buf(&self, size: usize) -> Result<FuseReplyBuf, FuseError> {
        self.device
            .from_device_pool
            .alloc_reply_buf(size)
            .map_err(FuseError::ResourceAlloc)
    }

    /// Allocates a buffer for `FUSE_WRITE` data.
    pub fn alloc_write_buf(&self, size: usize) -> Result<FuseRequestBuf, FuseError> {
        self.device
            .to_device_pool
            .alloc_request_buf(size)
            .map_err(FuseError::ResourceAlloc)
    }

    /// Returns the FUSE feature flags selected after negotiation.
    pub fn negotiated_flags(&self) -> FuseInitFlags {
        self.negotiated_flags
    }

    /// Returns the maximum write size accepted by the server.
    pub fn max_write(&self) -> u32 {
        self.max_write
    }

    fn init_flags() -> FuseInitFlags {
        FuseInitFlags::ASYNC_READ
            | FuseInitFlags::ATOMIC_O_TRUNC
            | FuseInitFlags::AUTO_INVAL_DATA
            | FuseInitFlags::BIG_WRITES
            | FuseInitFlags::HANDLE_KILLPRIV
            | FuseInitFlags::MAX_PAGES
            | FuseInitFlags::PARALLEL_DIROPS
            | FuseInitFlags::INIT_EXT
    }

    /// Sends a `FUSE_READ` request and waits for the reply.
    pub fn read(
        &self,
        nodeid: FuseNodeId,
        read_request: ReadReq,
        data_buf: FuseReplyBuf,
    ) -> Result<usize, FuseError> {
        let waiter = self.read_async(nodeid, read_request, data_buf, None)?;
        let read_len = waiter.wait().payload_len()?;
        if read_len > read_request.size() as usize {
            return Err(FuseError::MalformedResponse);
        }

        Ok(read_len)
    }

    /// Submits a `FUSE_READ` request without waiting for completion.
    ///
    /// Returns a waiter that the caller can await to obtain the reply.
    /// If `complete_fn` is provided, it is invoked after the
    /// server replies in a non-sleepable completion context.
    pub fn read_async(
        &self,
        nodeid: FuseNodeId,
        read_request: ReadReq,
        data_buf: FuseReplyBuf,
        complete_fn: Option<FuseCompleteFn>,
    ) -> Result<Arc<FuseWaiter>, FuseError> {
        let mut operation = ReadOperation::new(read_request);
        self.device.submit_fuse_op(
            nodeid,
            &mut operation,
            Some(FuseDataBuf::Read(data_buf)),
            complete_fn,
        )
    }

    /// Sends a `FUSE_READDIR` request and waits for the reply.
    pub fn readdir(
        &self,
        nodeid: FuseNodeId,
        read_request: ReadReq,
        data_buf: FuseReplyBuf,
    ) -> Result<Vec<FuseDirEntry>, FuseError> {
        let mut operation = ReaddirOperation::new(read_request);
        let waiter = self.device.submit_fuse_op(
            nodeid,
            &mut operation,
            Some(FuseDataBuf::Read(data_buf.clone())),
            None,
        )?;
        let payload_len = waiter.wait().payload_len()?;
        if payload_len > read_request.size() as usize {
            return Err(FuseError::MalformedResponse);
        }

        ReaddirOperation::parse_entries(payload_len, &mut data_buf.reader().unwrap())
    }

    /// Sends a `FUSE_READLINK` request and waits for the reply.
    pub fn readlink(&self, nodeid: FuseNodeId) -> Result<String, FuseError> {
        let data_buf = self.alloc_read_buf(MAX_READLINK_LEN)?;
        let mut operation = ReadlinkOperation;
        let waiter = self.device.submit_fuse_op(
            nodeid,
            &mut operation,
            Some(FuseDataBuf::Read(data_buf.clone())),
            None,
        )?;
        let payload_len = waiter.wait().payload_len()?;
        if payload_len > MAX_READLINK_LEN {
            return Err(FuseError::MalformedResponse);
        }

        ReadlinkOperation::parse_reply(payload_len, &mut data_buf.reader().unwrap())
    }

    /// Sends a `FUSE_WRITE` request and waits for the reply.
    pub fn write(
        &self,
        nodeid: FuseNodeId,
        write_request: WriteReq,
        data_buf: FuseRequestBuf,
    ) -> Result<usize, FuseError> {
        let waiter = self.write_async(nodeid, write_request, data_buf, None)?;
        let payload_len = waiter.wait().payload_len()?;

        let write_reply = waiter.parse_reply::<WriteOperation>(payload_len)?;
        if write_reply.size() > write_request.size() as usize {
            return Err(FuseError::MalformedResponse);
        }

        Ok(write_reply.size())
    }

    /// Submits a `FUSE_WRITE` request without waiting for completion.
    ///
    /// Returns a waiter that the caller can await to obtain the reply.
    /// If `complete_fn` is provided, it is invoked after the
    /// server replies in a non-sleepable completion context.
    pub fn write_async(
        &self,
        nodeid: FuseNodeId,
        write_request: WriteReq,
        data_buf: FuseRequestBuf,
        complete_fn: Option<FuseCompleteFn>,
    ) -> Result<Arc<FuseWaiter>, FuseError> {
        let mut operation = WriteOperation::new(write_request);
        self.device.submit_fuse_op(
            nodeid,
            &mut operation,
            Some(FuseDataBuf::Write(data_buf)),
            complete_fn,
        )
    }

    /// Sends a `FUSE_FORGET` request on the high-priority queue.
    ///
    /// `FUSE_FORGET` is a no-reply request. The server must not send a
    /// response, so this method only submits the request and never waits for
    /// completion.
    pub fn forget(&self, nodeid: FuseNodeId, nlookup: u64) -> Result<(), FuseError> {
        if nodeid == FUSE_ROOT_ID || nlookup == 0 {
            return Ok(());
        }

        let mut operation = ForgetOperation::new(ForgetReq::new(nlookup));
        let request = self
            .device
            .prepare_request(nodeid, &mut operation, None, None)?;
        self.device
            .submit(self.device.hiprio_queue.as_ref(), request)?;

        Ok(())
    }

    /// Releases the file or directory handle `fh` on `nodeid`.
    ///
    /// Returns request submission errors and server-reported release errors.
    /// If the filesystem has been unmounted, the release operation is skipped.
    pub fn release(
        &self,
        nodeid: FuseNodeId,
        fh: FuseFileHandle,
        flags: u32,
        release_options: ReleaseOptions,
    ) -> Result<(), FuseError> {
        self.do_fuse_op(nodeid, ReleaseOperation::new(fh, flags, release_options))
    }
}

/// Monotonically increasing token for FUSE client attribute-cache updates.
///
/// This is client-side session state used to reject stale attribute replies.
/// It is not part of the virtio-fs device protocol or the FUSE wire format.
///
/// Each FUSE request that may return attributes snapshots the current session
/// version before the request is sent. When the reply arrives, the snapshot is
/// compared against the inode's committed version: if the inode version is
/// strictly greater, a newer update has already committed and the stale reply
/// is discarded. Local metadata changes also bump the version.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct AttrVersion(u64);

impl AttrVersion {
    pub(super) const fn new(version: u64) -> Self {
        Self(version)
    }
}
