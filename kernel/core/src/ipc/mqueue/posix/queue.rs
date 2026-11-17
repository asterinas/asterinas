// SPDX-License-Identifier: MPL-2.0

//! Core data structures of a POSIX message queue.

use core::sync::atomic::AtomicUsize;

use ostd::sync::WaitQueue;

use crate::{
    fs::file::InodeMode,
    prelude::*,
    process::{Gid, Uid},
};

/// POSIX message queue attributes (`struct mq_attr`).
///
/// The field layout matches the user-space ABI of `struct mq_attr`, where each
/// field is a `long`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
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
/// shared between the IPC registry and every open file description.
pub(crate) struct MqueueQueue {
    /// The user who created the queue.
    ///
    /// Together with the sticky-bit rule of the `mqueuefs` root directory, the
    /// owner decides who may unlink the queue.
    owner: Uid,
    /// The group of the user who created the queue.
    group: Gid,
    /// Queue permission mode, fixed at creation time.
    mode: InodeMode,
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

impl MqueueQueue {
    /// Creates a new, empty message queue with the given owner, group,
    /// permission, and limits.
    pub(crate) fn new(
        mode: InodeMode,
        owner: Uid,
        group: Gid,
        mq_maxmsg: i64,
        mq_msgsize: i64,
    ) -> Self {
        Self {
            owner,
            group,
            mode,
            attr: MqAttr {
                mq_flags: 0,
                mq_maxmsg,
                mq_msgsize,
                mq_curmsgs: 0,
            },
            messages: Mutex::new(BTreeMap::new()),
            qsize: AtomicUsize::new(0),
            wait_queue: WaitQueue::new(),
        }
    }

    /// Returns the queue permission mode.
    pub(crate) fn mode(&self) -> InodeMode {
        self.mode
    }

    /// Returns the user who created the queue.
    pub(crate) fn owner(&self) -> Uid {
        self.owner
    }

    /// Returns the group of the user who created the queue.
    pub(crate) fn group(&self) -> Gid {
        self.group
    }

    /// Returns a snapshot of the queue attributes.
    ///
    /// `mq_flags` is always zero here, because `O_NONBLOCK` is a property of
    /// each open file description rather than of the queue itself.
    pub(crate) fn attr(&self) -> MqAttr {
        let messages = self.messages.lock();
        let mq_curmsgs = messages.values().map(|msgs| msgs.len()).sum::<usize>() as i64;

        MqAttr {
            mq_flags: 0,
            mq_maxmsg: self.attr.mq_maxmsg,
            mq_msgsize: self.attr.mq_msgsize,
            mq_curmsgs,
        }
    }
}
