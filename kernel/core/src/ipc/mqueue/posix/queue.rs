// SPDX-License-Identifier: MPL-2.0

//! Core data structures of a POSIX message queue.

use core::sync::atomic::{AtomicI64, AtomicUsize, Ordering};

use ostd::sync::WaitQueue;

use crate::{
    fs::file::InodeMode,
    prelude::*,
    process::{Gid, Uid},
    time::wait::ManagedTimeout,
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
    /// Number of messages currently queued (`mq_curmsgs`).
    ///
    /// This mirrors the total length of `messages` so that reads avoid an
    /// `O(n)` count over the priority buckets.
    curmsgs: AtomicI64,
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
            curmsgs: AtomicI64::new(0),
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
        MqAttr {
            mq_flags: 0,
            mq_maxmsg: self.attr.mq_maxmsg,
            mq_msgsize: self.attr.mq_msgsize,
            mq_curmsgs: self.curmsgs.load(Ordering::Relaxed),
        }
    }

    /// Returns the total number of payload bytes currently queued.
    pub(crate) fn qsize(&self) -> usize {
        self.qsize.load(Ordering::Relaxed)
    }

    /// Sends a message to the queue.
    ///
    /// If the queue is full, this method blocks until space becomes available,
    /// the timeout expires, or the current thread is interrupted by a signal.
    pub(crate) fn send(
        &self,
        payload: Box<[u8]>,
        prio: u32,
        nonblock: bool,
        timeout: Option<ManagedTimeout<'static>>,
    ) -> Result<()> {
        let mut payload = Some(payload);

        if self.try_send(&mut payload, prio) {
            self.wait_queue.wake_all();
            return Ok(());
        }

        if nonblock {
            return_errno!(Errno::EAGAIN);
        }

        let res = self
            .wait_queue
            .pause_until_or_timeout(|| self.try_send(&mut payload, prio).then_some(()), timeout);

        match res {
            Ok(()) => {
                self.wait_queue.wake_all();
                Ok(())
            }
            Err(err) if err.error() == Errno::ETIME => return_errno!(Errno::ETIMEDOUT),
            Err(err) => Err(err),
        }
    }

    /// Receives the highest-priority message from the queue.
    ///
    /// If `msg_len` is smaller than the queued message, `EMSGSIZE` is returned
    /// and the message stays queued. If the queue is empty, this method blocks
    /// until a message arrives, the timeout expires, or the current thread is
    /// interrupted by a signal.
    pub(crate) fn receive(
        &self,
        msg_len: usize,
        nonblock: bool,
        timeout: Option<ManagedTimeout<'static>>,
    ) -> Result<(Box<[u8]>, u32)> {
        match self.try_receive(msg_len) {
            ReceiveOutcome::Success(msg) => {
                self.wait_queue.wake_all();
                return Ok(msg);
            }
            ReceiveOutcome::TooLarge => return_errno!(Errno::EMSGSIZE),
            ReceiveOutcome::Empty => {}
        }

        if nonblock {
            return_errno!(Errno::EAGAIN);
        }

        let res = self.wait_queue.pause_until_or_timeout(
            || match self.try_receive(msg_len) {
                ReceiveOutcome::Success(msg) => Some(Ok(msg)),
                ReceiveOutcome::TooLarge => Some(Err(Error::new(Errno::EMSGSIZE))),
                ReceiveOutcome::Empty => None,
            },
            timeout,
        );

        match res {
            Ok(Ok(msg)) => {
                self.wait_queue.wake_all();
                Ok(msg)
            }
            Ok(Err(err)) => Err(err),
            Err(err) if err.error() == Errno::ETIME => return_errno!(Errno::ETIMEDOUT),
            Err(err) => Err(err),
        }
    }

    fn try_send(&self, payload: &mut Option<Box<[u8]>>, prio: u32) -> bool {
        let mut messages = self.messages.lock();
        if self.curmsgs.load(Ordering::Relaxed) >= self.attr.mq_maxmsg {
            return false;
        }

        let payload = payload.take().unwrap();
        let len = payload.len();
        messages.entry(prio).or_default().push_back(MqMsg {
            priority: prio,
            payload,
        });
        self.curmsgs.fetch_add(1, Ordering::Relaxed);
        self.qsize.fetch_add(len, Ordering::Relaxed);
        true
    }

    fn try_receive(&self, msg_len: usize) -> ReceiveOutcome {
        let mut messages = self.messages.lock();

        let Some(mut entry) = messages.last_entry() else {
            return ReceiveOutcome::Empty;
        };

        // Peek without consuming so that an oversized message stays queued.
        let first_len = entry.get().front().unwrap().payload.len();
        if first_len > msg_len {
            return ReceiveOutcome::TooLarge;
        }

        let msg = entry.get_mut().pop_front().unwrap();
        if entry.get().is_empty() {
            entry.remove();
        }

        let len = msg.payload.len();
        self.curmsgs.fetch_sub(1, Ordering::Relaxed);
        self.qsize.fetch_sub(len, Ordering::Relaxed);

        ReceiveOutcome::Success((msg.payload, msg.priority))
    }
}

/// The outcome of an attempt to receive a message.
enum ReceiveOutcome {
    Success((Box<[u8]>, u32)),
    TooLarge,
    Empty,
}
