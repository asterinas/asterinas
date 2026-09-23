// SPDX-License-Identifier: MPL-2.0

//! Waitable reply state for submitted FUSE requests.
//!
//! This module defines [`FuseWaiter`], which lets synchronous callers block for
//! a FUSE reply and lets asynchronous users integrate a request into an
//! [`io_util::batch::IoBatch`].

use aster_fuse::{FuseCompletion, FuseError, FuseOperation, FuseStatus, ReplyHeader};
use io_util::{IoError, batch::IoCompletion};
use ostd::{
    mm::io::util::HasVmReaderWriter,
    sync::{LocalIrqDisabled, SpinLock, WaitQueue},
};
use smallvec::SmallVec;

use crate::device::filesystem::pool::FuseReplyBuf;

/// Reply buffers owned by one submitted FUSE request.
pub(super) struct ReplyBufs {
    header: Option<FuseReplyBuf>,
    payload: SmallVec<[FuseReplyBuf; 1]>,
}

impl ReplyBufs {
    /// Creates a reply-buffer set with an optional reply header.
    pub(super) fn new(header: Option<FuseReplyBuf>) -> Self {
        Self {
            header,
            payload: SmallVec::new(),
        }
    }

    /// Adds a payload buffer after the reply header.
    pub(super) fn push_payload(&mut self, payload: FuseReplyBuf) {
        debug_assert!(self.header.is_some());
        self.payload.push(payload);
    }

    /// Returns whether no reply buffer is expected.
    pub(super) fn is_empty(&self) -> bool {
        self.header.is_none()
    }

    /// Returns the FUSE reply header buffer.
    pub(super) fn reply_header_buf(&self) -> Result<&FuseReplyBuf, FuseError> {
        self.header.as_ref().ok_or(FuseError::MalformedResponse)
    }

    /// Returns payload buffers in virtqueue descriptor order.
    pub(super) fn payload_bufs(&self) -> &[FuseReplyBuf] {
        &self.payload
    }

    /// Returns the reply buffers in virtqueue descriptor order.
    pub(super) fn iter(&self) -> impl Iterator<Item = &FuseReplyBuf> {
        self.header.iter().chain(self.payload.iter())
    }

    /// Returns the only payload buffer used by typed reply parsers.
    ///
    /// `FUSE_INIT`, `FUSE_WRITE`, and typed replies reached through
    /// `FuseSession::do_fuse_op` use one payload buffer after the reply header.
    /// A `FUSE_READ` reply may use several page buffers, but
    /// [`FuseOperation::parse_reply`] is not a scatter-gather parser. Rejecting
    /// more than one payload prevents it from silently parsing only the first
    /// fragment.
    fn single_payload(&self) -> Result<Option<&FuseReplyBuf>, FuseError> {
        match self.payload_bufs() {
            [] => Ok(None),
            [payload] => Ok(Some(payload)),
            [..] => Err(FuseError::MalformedResponse),
        }
    }
}

/// A waiter for one submitted FUSE request.
#[must_use]
pub struct FuseWaiter {
    /// Reply buffers in virtqueue descriptor order. The first buffer is the
    /// [`ReplyHeader`] whenever a reply is expected.
    reply_bufs: ReplyBufs,
    status: SpinLock<FuseStatus, LocalIrqDisabled>,
    wait_queue: WaitQueue,
}

impl FuseWaiter {
    /// Creates a waiter for a request's reply buffers.
    pub(super) fn new(reply_bufs: ReplyBufs) -> Self {
        Self {
            reply_bufs,
            status: SpinLock::new(FuseStatus::Pending),
            wait_queue: WaitQueue::new(),
        }
    }

    /// Parses a typed FUSE operation reply from the payload bytes.
    pub(super) fn parse_reply<Op: FuseOperation>(
        &self,
        payload_len: usize,
    ) -> Result<Op::Output, FuseError> {
        let mut reader = if let Some(payload_buf) = self.reply_bufs.single_payload()? {
            payload_buf.reader().unwrap()
        } else {
            let header_buf = self.reply_bufs.reply_header_buf()?;
            let mut reader = header_buf.reader().unwrap();
            reader.skip(size_of::<ReplyHeader>());
            reader
        };

        Op::parse_reply(payload_len, &mut reader)
    }

    /// Waits until the FUSE request completes.
    ///
    /// # Locking
    ///
    /// This method may sleep. Callers must not call it while holding a
    /// spinlock, IRQ-disabled guard, or any other lock that cannot be held
    /// across sleep.
    pub(super) fn wait(&self) -> FuseCompletion {
        // FIXME: There is no timeout logic. If the host virtio-fs server stalls,
        // the guest driver task will block indefinitely. Adding timeout support
        // is non-trivial: simply dropping the in-flight request is not safe,
        // because the host may still hold descriptors pointing to the guest's
        // DMA buffers. A proper timeout path would require restoring the
        // virtqueue state, which in turn likely necessitates a full device reset.
        self.wait_queue.wait_until(|| {
            let status = *self.status.lock();
            status.has_completed()
        })
    }

    /// Records completion and wakes waiters.
    pub(super) fn wake_completed(&self, completion: FuseCompletion) {
        let should_wake = {
            let mut current_status = self.status.lock();
            if !current_status.is_pending() {
                false
            } else {
                *current_status = FuseStatus::Completed(completion);
                true
            }
        };

        if should_wake {
            self.wait_queue.wake_all();
        }
    }

    /// Returns reply DMA buffers that hold the FUSE reply.
    pub(super) fn reply_bufs(&self) -> &ReplyBufs {
        &self.reply_bufs
    }
}

impl IoCompletion for FuseWaiter {
    fn wait(&self) -> Result<(), IoError> {
        // TODO: Preserve the number of bytes accepted by a short write for
        // callers that need partial-write semantics.
        match self.wait() {
            FuseCompletion::Complete(_) | FuseCompletion::ShortWrite { .. } => Ok(()),
            FuseCompletion::MalformedResponse | FuseCompletion::RemoteError(_) => {
                Err(IoError::Failed)
            }
        }
    }
}
