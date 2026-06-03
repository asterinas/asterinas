// SPDX-License-Identifier: MPL-2.0

//! FUSE requests for virtio-fs queues.
//!
//! This module defines [`FuseRequest`], the state kept from virtqueue
//! submission until the device returns the descriptor. A request keeps DMA
//! buffers alive, records the FUSE request identity used to validate replies,
//! and reports the final [`FuseCompletion`] to the optional completion callback
//! and the [`FuseWaiter`].

use alloc::sync::Arc;

use aster_fuse::{FuseCompleteFn, FuseCompletion, FuseNodeId, FuseUnique, ReplyHeader};
use ostd::mm::io::util::HasVmReaderWriter;
use smallvec::SmallVec;

use super::waiter::{FuseWaiter, ReplyBufs};
use crate::device::filesystem::pool::FuseRequestBuf;

/// DMA buffers sent to the device as one FUSE request.
pub(super) type RequestBufs = SmallVec<[FuseRequestBuf; 2]>;

/// State for one submitted FUSE request while its descriptor is in flight.
pub(super) struct FuseRequest {
    unique: FuseUnique,
    nodeid: FuseNodeId,
    request_bufs: RequestBufs,
    waiter: Arc<FuseWaiter>,
    complete_fn: Option<FuseCompleteFn>,
}

impl FuseRequest {
    /// Creates a submitted request state from outbound and reply DMA buffers.
    ///
    /// The returned request owns the DMA buffers until the virtqueue reports the
    /// descriptor as used. `complete_fn`, when present, is called after the
    /// request completion has been classified.
    pub(super) fn new(
        unique: FuseUnique,
        nodeid: FuseNodeId,
        request_bufs: RequestBufs,
        reply_bufs: ReplyBufs,
        complete_fn: Option<FuseCompleteFn>,
    ) -> Self {
        Self {
            unique,
            nodeid,
            request_bufs,
            waiter: Arc::new(FuseWaiter::new(reply_bufs)),
            complete_fn,
        }
    }

    /// Completes the request from the used-descriptor reply length.
    ///
    /// This method synchronizes reply buffers from the device when a reply is
    /// expected, classifies the result, invokes the optional completion
    /// callback, and wakes waiters with the resulting [`FuseCompletion`].
    pub(super) fn finish_reply(mut self, reply_len: usize) {
        let completion = self.parse_reply_header(reply_len);
        if let Some(complete_fn) = self.complete_fn.take() {
            complete_fn(completion);
        }

        self.waiter.wake_completed(completion);
    }

    /// Returns the FUSE node ID used to choose the request queue.
    pub(super) fn nodeid(&self) -> FuseNodeId {
        self.nodeid
    }

    /// Returns the outbound DMA buffers submitted to the device.
    pub(super) fn request_bufs(&self) -> &[FuseRequestBuf] {
        self.request_bufs.as_slice()
    }

    /// Returns the number of virtqueue descriptors needed by this request.
    pub(super) fn num_dma_bufs(&self) -> usize {
        self.request_bufs.len() + self.waiter.reply_bufs().iter().count()
    }

    /// Returns the waiter that owns the reply buffers and completion status.
    pub(super) fn waiter(&self) -> &Arc<FuseWaiter> {
        &self.waiter
    }

    /// Parses the common reply header into a request completion status.
    fn parse_reply_header(&self, reply_len: usize) -> FuseCompletion {
        let reply_bufs = self.waiter.reply_bufs();
        if reply_bufs.is_empty() {
            return FuseCompletion::Complete(0);
        }
        if reply_len < size_of::<ReplyHeader>() {
            return FuseCompletion::MalformedResponse;
        }
        for reply_buf in reply_bufs.iter() {
            reply_buf.sync_from_device().unwrap();
        }

        let Some(reply_header_buf) = reply_bufs.header() else {
            return FuseCompletion::MalformedResponse;
        };

        let mut reader = reply_header_buf.reader().unwrap();
        let Ok(reply_header) = reader.read_val::<ReplyHeader>() else {
            return FuseCompletion::MalformedResponse;
        };

        // TODO: Short `FUSE_READ` and `FUSE_READDIR` replies are not supported
        // here. If `ReplyHeader::len` reports fewer bytes than the virtqueue used
        // length, the response is treated as malformed.
        if reply_header.len() as usize != reply_len || reply_header.unique() != self.unique {
            return FuseCompletion::MalformedResponse;
        }

        if reply_header.error() != 0 {
            return FuseCompletion::RemoteError(reply_header.error());
        }

        if let Some(payload_len) =
            (reply_header.len() as usize).checked_sub(size_of::<ReplyHeader>())
        {
            FuseCompletion::Complete(payload_len)
        } else {
            FuseCompletion::MalformedResponse
        }
    }
}
