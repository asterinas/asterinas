// SPDX-License-Identifier: MPL-2.0

//! Core data structures of a POSIX message queue.

use core::sync::atomic::{AtomicI64, AtomicUsize, Ordering};

use ostd::sync::WaitQueue;

use crate::{
    fs::file::InodeMode,
    prelude::*,
    process::{
        Gid, Pid, Process, Uid,
        posix_thread::AsPosixThread,
        signal::{
            c_types::{siginfo_t, sigval_t},
            constants::SI_MESGQ,
            sig_num::SigNum,
            signals::Signal,
        },
    },
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
///
/// # Locking
///
/// The only lock nesting in this type is `messages` and then `notify`:
/// [`Self::try_send`] takes `messages` first and may then take `notify` to
/// consume a pending one-shot notification. Nothing takes `notify` and then
/// `messages`, so there is no lock cycle. `wait_queue` is never held while
/// taking either of the two locks.
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
    /// This mirrors the total number of messages across the priority buckets of
    /// `messages`, so that reads avoid an `O(n)` count on the hot path. It is
    /// updated only while holding `messages`; readers that do not hold the lock
    /// (e.g. [`attr`](Self::attr)) may observe a value that is briefly
    /// inconsistent with `qsize`, which is acceptable because the two counters
    /// are queried independently.
    curmsgs: AtomicI64,
    /// Messages currently queued, grouped by priority.
    ///
    /// The key is the message priority and the value is a FIFO queue of the
    /// messages with that priority. Higher priorities are dequeued first.
    messages: Mutex<BTreeMap<u32, VecDeque<MqMsg>>>,
    /// Total number of payload bytes currently queued.
    ///
    /// Like `curmsgs`, it is updated only while holding `messages` and may be
    /// read without the lock; a lock-free read can therefore be briefly stale.
    qsize: AtomicUsize,
    /// Wait queue for blocking `mq_timedsend` / `mq_timedreceive`.
    wait_queue: WaitQueue,
    /// One-shot notification registration for `mq_notify`, if any.
    notify: Mutex<Option<MqNotify>>,
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
            notify: Mutex::new(None),
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

    /// Registers a one-shot notification for this queue.
    ///
    /// Only one process may register a notification at a time. `signal` is the
    /// signal to deliver for `SIGEV_SIGNAL` and is `None` for `SIGEV_NONE`.
    pub(crate) fn register_notify(
        &self,
        owner: Pid,
        signal: Option<(SigNum, Arc<Process>, sigval_t)>,
    ) -> Result<()> {
        let mut notify = self.notify.lock();
        if notify.is_some() {
            return Err(errno!(EBUSY));
        }
        *notify = Some(MqNotify { owner, signal });
        Ok(())
    }

    /// Cancels the notification registered by `owner`.
    ///
    /// Matching Linux, an attempt to cancel a notification registered by
    /// another process is silently ignored rather than reported as an error.
    pub(crate) fn unregister_notify(&self, owner: Pid) {
        let mut notify = self.notify.lock();
        if notify
            .as_ref()
            .is_some_and(|registered| registered.owner == owner)
        {
            *notify = None;
        }
    }

    /// Fires a pending one-shot notification, if any, and wakes up waiters.
    ///
    /// The notification is fired outside any lock, because delivering a signal
    /// may itself acquire locks.
    fn fire_notify_and_wake(&self, notify: Option<MqNotify>) {
        if let Some(notify) = notify {
            notify.fire();
        }
        self.wait_queue.wake_all();
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
        let mut payload = payload;

        // Fast path: enqueue immediately when there is room.
        match self.try_send(payload, prio) {
            TrySendOutcome::Enqueued { notify } => {
                self.fire_notify_and_wake(notify);
                return Ok(());
            }
            TrySendOutcome::Full(returned) => {
                payload = returned;
            }
        }

        if nonblock {
            return Err(errno!(EAGAIN));
        }

        // Slow path: block until room is available. The payload is carried
        // across retries so that it is never lost while the queue stays full.
        let res = self.wait_queue.pause_until_or_timeout(
            move || match self.try_send(core::mem::take(&mut payload), prio) {
                TrySendOutcome::Enqueued { notify } => Some(notify),
                TrySendOutcome::Full(returned) => {
                    payload = returned;
                    None
                }
            },
            timeout,
        );

        let notify = match res {
            Ok(notify) => notify,
            Err(err) if err.error() == Errno::ETIME => return Err(errno!(ETIMEDOUT)),
            Err(err) => return Err(err),
        };

        self.fire_notify_and_wake(notify);
        Ok(())
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
            ReceiveOutcome::TooLarge => return Err(errno!(EMSGSIZE)),
            ReceiveOutcome::Empty => {}
        }

        if nonblock {
            return Err(errno!(EAGAIN));
        }

        let res = self.wait_queue.pause_until_or_timeout(
            || match self.try_receive(msg_len) {
                ReceiveOutcome::Success(msg) => Some(Ok(msg)),
                ReceiveOutcome::TooLarge => Some(Err(errno!(EMSGSIZE))),
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
            Err(err) if err.error() == Errno::ETIME => Err(errno!(ETIMEDOUT)),
            Err(err) => Err(err),
        }
    }

    fn try_send(&self, payload: Box<[u8]>, prio: u32) -> TrySendOutcome {
        let mut messages = self.messages.lock();
        if self.curmsgs.load(Ordering::Relaxed) >= self.attr.mq_maxmsg {
            return TrySendOutcome::Full(payload);
        }

        // The queue transitions from empty to non-empty, so a one-shot
        // notification (if registered) is fired.
        let was_empty = self.curmsgs.load(Ordering::Relaxed) == 0;

        let len = payload.len();
        messages.entry(prio).or_default().push_back(MqMsg {
            priority: prio,
            payload,
        });
        self.curmsgs.fetch_add(1, Ordering::Relaxed);
        self.qsize.fetch_add(len, Ordering::Relaxed);

        let notify = if was_empty {
            // The notification is one-shot, so it is consumed by this
            // transition.
            self.notify.lock().take()
        } else {
            None
        };

        TrySendOutcome::Enqueued { notify }
    }

    fn try_receive(&self, msg_len: usize) -> ReceiveOutcome {
        let mut messages = self.messages.lock();

        loop {
            let Some(mut entry) = messages.last_entry() else {
                return ReceiveOutcome::Empty;
            };

            // Every bucket must be non-empty, but handle an empty bucket
            // without panicking just in case: drop it and look at the next one.
            let Some(first) = entry.get().front() else {
                entry.remove();
                continue;
            };

            // Peek without consuming so that an oversized message stays queued.
            let first_len = first.payload.len();
            if first_len > msg_len {
                return ReceiveOutcome::TooLarge;
            }

            let Some(msg) = entry.get_mut().pop_front() else {
                entry.remove();
                continue;
            };
            if entry.get().is_empty() {
                entry.remove();
            }

            let len = msg.payload.len();
            self.curmsgs.fetch_sub(1, Ordering::Relaxed);
            self.qsize.fetch_sub(len, Ordering::Relaxed);

            return ReceiveOutcome::Success((msg.payload, msg.priority));
        }
    }
}

/// The outcome of a send attempt.
enum TrySendOutcome {
    /// The queue was full; the payload is returned so the caller can retry.
    Full(Box<[u8]>),
    /// The message was enqueued.
    Enqueued {
        /// The one-shot notification to fire, if the queue was previously
        /// empty and a notification was registered.
        notify: Option<MqNotify>,
    },
}

/// The outcome of an attempt to receive a message.
enum ReceiveOutcome {
    Success((Box<[u8]>, u32)),
    TooLarge,
    Empty,
}

/// A one-shot `mq_notify` registration.
struct MqNotify {
    /// The process that registered the notification.
    owner: Pid,
    /// The signal to deliver for `SIGEV_SIGNAL`; `None` for `SIGEV_NONE`.
    signal: Option<(SigNum, Arc<Process>, sigval_t)>,
}

impl MqNotify {
    /// Fires the notification, consuming it.
    fn fire(self) {
        if let Some((signo, process, value)) = self.signal {
            // The notification is delivered by the thread that made the queue
            // non-empty. If the current task is not a POSIX thread, there is no
            // pid/uid to report, so the notification is dropped rather than
            // panicking.
            let Some(current) = crate::thread::Thread::current() else {
                return;
            };
            let Some(posix_thread) = current.as_posix_thread() else {
                return;
            };

            let pid = posix_thread.process().pid();
            let uid = posix_thread.credentials().euid();
            let signal = MqNotifySignal::new(signo, value, pid, uid);
            process.enqueue_signal(Box::new(signal));
        }
    }
}

/// The signal delivered for a `SIGEV_SIGNAL` notification.
struct MqNotifySignal {
    num: SigNum,
    value: sigval_t,
    pid: Pid,
    uid: Uid,
}

impl Debug for MqNotifySignal {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("MqNotifySignal")
            .field("num", &self.num)
            .field("pid", &self.pid)
            .field("uid", &self.uid)
            .finish()
    }
}

impl MqNotifySignal {
    fn new(num: SigNum, value: sigval_t, pid: Pid, uid: Uid) -> Self {
        Self {
            num,
            value,
            pid,
            uid,
        }
    }
}

impl Signal for MqNotifySignal {
    fn num(&self) -> SigNum {
        self.num
    }

    fn to_info(&self) -> siginfo_t {
        let mut info = siginfo_t::new(self.num, SI_MESGQ);
        info.set_ptr(self.value.read_ptr());
        info.set_pid_uid(self.pid, self.uid);
        info
    }
}

#[cfg(ktest)]
mod test {
    use ostd::prelude::*;

    use super::*;
    use crate::thread::{Thread, kernel_thread::ThreadOptions};

    /// Creates a queue owned by root with `0o644` permissions.
    fn new_queue(mq_maxmsg: i64, mq_msgsize: i64) -> MqueueQueue {
        MqueueQueue::new(
            InodeMode::from_bits_truncate(0o644),
            Uid::new_root(),
            Gid::new_root(),
            mq_maxmsg,
            mq_msgsize,
        )
    }

    /// Converts a byte slice into an owned payload.
    fn payload(bytes: &[u8]) -> Box<[u8]> {
        Box::from(bytes)
    }

    #[ktest]
    fn send_receive_fifo() {
        let queue = new_queue(4, 8);
        queue.send(payload(&[1u8, 2, 3]), 0, false, None).unwrap();
        queue.send(payload(&[4u8, 5]), 0, false, None).unwrap();

        let (msg, prio) = queue.receive(8, false, None).unwrap();
        assert_eq!(prio, 0);
        assert_eq!(&*msg, [1u8, 2, 3].as_slice());

        let (msg, prio) = queue.receive(8, false, None).unwrap();
        assert_eq!(prio, 0);
        assert_eq!(&*msg, [4u8, 5].as_slice());
    }

    #[ktest]
    fn receive_by_priority() {
        let queue = new_queue(4, 8);
        queue.send(payload(&[1u8]), 1, false, None).unwrap();
        queue.send(payload(&[2u8]), 5, false, None).unwrap();
        queue.send(payload(&[3u8]), 3, false, None).unwrap();

        assert_eq!(queue.receive(8, false, None).unwrap().1, 5);
        assert_eq!(queue.receive(8, false, None).unwrap().1, 3);
        assert_eq!(queue.receive(8, false, None).unwrap().1, 1);
    }

    #[ktest]
    fn send_nonblock_full() {
        let queue = new_queue(1, 8);
        queue.send(payload(&[1u8]), 0, false, None).unwrap();

        assert_eq!(
            queue
                .send(payload(&[2u8]), 0, true, None)
                .unwrap_err()
                .error(),
            Errno::EAGAIN
        );
    }

    #[ktest]
    fn receive_nonblock_empty() {
        let queue = new_queue(4, 8);
        assert_eq!(
            queue.receive(8, true, None).unwrap_err().error(),
            Errno::EAGAIN
        );
    }

    #[ktest]
    fn receive_emsgsize_keeps_message() {
        let queue = new_queue(4, 4);
        queue
            .send(payload(&[1u8, 2, 3, 4]), 0, false, None)
            .unwrap();

        // A too-small buffer must not consume the message.
        assert_eq!(
            queue.receive(2, true, None).unwrap_err().error(),
            Errno::EMSGSIZE
        );

        let (msg, _) = queue.receive(4, false, None).unwrap();
        assert_eq!(&*msg, [1u8, 2, 3, 4].as_slice());
    }

    #[ktest]
    fn attr_and_qsize() {
        let queue = new_queue(4, 8);
        let attr = queue.attr();
        assert_eq!(attr.mq_maxmsg, 4);
        assert_eq!(attr.mq_msgsize, 8);
        assert_eq!(attr.mq_curmsgs, 0);
        assert_eq!(queue.qsize(), 0);

        queue.send(payload(&[1u8, 2, 3]), 0, false, None).unwrap();
        assert_eq!(queue.attr().mq_curmsgs, 1);
        assert_eq!(queue.qsize(), 3);

        queue.receive(8, false, None).unwrap();
        assert_eq!(queue.attr().mq_curmsgs, 0);
        assert_eq!(queue.qsize(), 0);
    }

    #[ktest]
    fn blocking_send() {
        let queue = Arc::new(new_queue(1, 8));
        queue.send(payload(&[1u8]), 0, false, None).unwrap();

        let queue_clone = queue.clone();
        let receiver = ThreadOptions::new(move || {
            Thread::yield_now();
            queue_clone.receive(8, false, None).unwrap();
        })
        .spawn();

        // Blocks until the receiver drains the queue.
        queue.send(payload(&[2u8]), 0, false, None).unwrap();

        receiver.join();
        assert_eq!(queue.attr().mq_curmsgs, 1);
    }

    #[ktest]
    fn blocking_receive() {
        let queue = Arc::new(new_queue(4, 8));

        let queue_clone = queue.clone();
        let sender = ThreadOptions::new(move || {
            Thread::yield_now();
            queue_clone
                .send(payload(&[1u8, 2]), 0, false, None)
                .unwrap();
        })
        .spawn();

        let (msg, prio) = queue.receive(8, false, None).unwrap();
        assert_eq!(prio, 0);
        assert_eq!(&*msg, [1u8, 2].as_slice());

        sender.join();
    }

    #[ktest]
    fn notify_register_unregister() {
        let queue = new_queue(4, 8);

        queue.register_notify(1, None).unwrap();
        assert_eq!(
            queue.register_notify(2, None).unwrap_err().error(),
            Errno::EBUSY
        );

        // Cancelling another owner's notification is a no-op.
        queue.unregister_notify(3);
        assert_eq!(
            queue.register_notify(2, None).unwrap_err().error(),
            Errno::EBUSY
        );

        // Cancelling the correct owner frees the slot.
        queue.unregister_notify(1);
        queue.register_notify(2, None).unwrap();
    }
}
