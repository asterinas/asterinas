// SPDX-License-Identifier: MPL-2.0

//! Core data structures of a POSIX message queue.

use core::sync::atomic::AtomicUsize;

use ostd::sync::WaitQueue;

use crate::prelude::*;

/// POSIX message queue attributes (`struct mq_attr`).
///
/// The field layout matches the user-space ABI of `struct mq_attr`, where each
/// field is a `long`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct MqAttr {
    /// Message queue flags; only `O_NONBLOCK` is meaningful.
    pub(crate) mq_flags: i64,
    /// Maximum number of messages allowed in the queue.
    pub(crate) mq_maxmsg: i64,
    /// Maximum size of a single message.
    pub(crate) mq_msgsize: i64,
    /// Number of messages currently queued.
    pub(crate) mq_curmsgs: i64,
}

/// A single message stored in a POSIX message queue.
pub(crate) struct MqMsg {
    /// Message priority. Higher values are dequeued first.
    priority: u32,
    /// Message payload.
    payload: Box<[u8]>,
}

/// The core state of a POSIX message queue.
///
/// This is the in-memory representation of a queue created by `mq_open`. It is
/// shared between the queue's VFS inode and every open file description.
pub(crate) struct MqueueQueue {
    /// Queue attributes.
    attr: MqAttr,
    /// Messages currently queued, grouped by priority.
    ///
    /// The key is the message priority and the value is a FIFO queue of the
    /// messages with that priority. Higher priorities are dequeued first.
    messages: Mutex<BTreeMap<u32, VecDeque<MqMsg>>>,
    /// Total number of payload bytes currently queued.
    qsize: AtomicUsize,
    /// Wait queue for blocking `mq_timedsend` / `mq_timedreceive`.
    wait_queue: WaitQueue,
    // TODO: Add `mq_notify` registration (`sigevent` and the owning thread/process).
}
