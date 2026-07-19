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

use crate::device::filesystem::pool::FuseReplyBuf;

/// Reply buffers owned by one submitted FUSE request.
pub(super) struct ReplyBufs {
    header: Option<FuseReplyBuf>,
    payload: Option<FuseReplyBuf>,
}

impl ReplyBufs {
    /// Creates an empty buffer set for a request that expects no reply.
    pub(super) fn new_none() -> Self {
        Self {
            header: None,
            payload: None,
        }
    }

    /// Creates a buffer set for a reply that only contains a `ReplyHeader`.
    pub(super) fn new_header_only(header: FuseReplyBuf) -> Self {
        Self {
            header: Some(header),
            payload: None,
        }
    }

    /// Creates a buffer set for a reply with a `ReplyHeader` and payload.
    pub(super) fn new_with_payload(header: FuseReplyBuf, payload: FuseReplyBuf) -> Self {
        Self {
            header: Some(header),
            payload: Some(payload),
        }
    }

    /// Returns whether no reply buffer is expected.
    pub(super) fn is_empty(&self) -> bool {
        self.header.is_none()
    }

    /// Returns the reply header buffer.
    pub(super) fn header(&self) -> Option<&FuseReplyBuf> {
        self.header.as_ref()
    }

    /// Returns the reply payload buffer.
    pub(super) fn payload(&self) -> Option<&FuseReplyBuf> {
        self.payload.as_ref()
    }

    /// Returns the reply buffers in virtqueue descriptor order.
    pub(super) fn iter(&self) -> impl Iterator<Item = &FuseReplyBuf> {
        self.header.iter().chain(self.payload.iter())
    }
}

/// A waiter for one submitted FUSE request.
#[must_use]
pub struct FuseWaiter {
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

    fn reply_header_buf(&self) -> Result<&FuseReplyBuf, FuseError> {
        let Some(header_buf) = self.reply_bufs.header() else {
            return Err(FuseError::MalformedResponse);
        };
        Ok(header_buf)
    }

    /// Parses a typed FUSE operation reply from the payload bytes.
    pub(super) fn parse_reply<Op: FuseOperation>(
        &self,
        payload_len: usize,
    ) -> Result<Op::Output, FuseError> {
        let mut reader = if let Some(payload_buf) = self.reply_bufs.payload() {
            payload_buf.reader().unwrap()
        } else {
            let header_buf = self.reply_header_buf()?;
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
        match self.wait() {
            FuseCompletion::Complete(_) => Ok(()),
            FuseCompletion::MalformedResponse | FuseCompletion::RemoteError(_) => {
                Err(IoError::Failed)
            }
        }
    }
}
