// SPDX-License-Identifier: MPL-2.0

use alloc::{
    collections::{btree_map::BTreeMap, vec_deque::VecDeque},
    sync::Arc,
};
use core::sync::atomic::{AtomicBool, Ordering};

use ostd::sync::{RwLock, WaitQueue};
use spin::Once;

use crate::{
    events::IoEvents,
    net::socket::{
        unix::{
            addr::{UnixSocketAddrBound, UnixSocketAddrKey},
            ctrl_msg::AuxiliaryData,
            UnixSocketAddr,
        },
        util::ControlMessage,
    },
    prelude::*,
    process::signal::Pollee,
    util::{MultiRead, MultiWrite},
};

pub(super) struct MessageQueue {
    addr: Once<UnixSocketAddr>,
    inner: Mutex<Option<Inner>>,
    is_pass_cred: AtomicBool,
    pollee: Pollee,
    send_wait_queue: WaitQueue,
}

struct Inner {
    messages: VecDeque<Message>,
    total_length: usize,
    is_shutdown: bool,
}

struct Message {
    bytes: Vec<u8>,
    aux: AuxiliaryData,
    src: UnixSocketAddr,
}

impl MessageQueue {
    /// Looks up a message queue bound to the specific address in the global table.
    pub(super) fn lookup_bound(addr: &UnixSocketAddrKey) -> Result<Arc<MessageQueue>> {
        QUEUE_TABLE.get_queue(addr).ok_or_else(|| {
            Error::with_message(Errno::ECONNREFUSED, "the target socket does not exist")
        })
    }

    pub(super) fn try_send(
        &self,
        reader: &mut dyn MultiRead,
        aux_data: &mut AuxiliaryData,
        source: &MessageReceiver,
    ) -> Result<usize> {
        let mut inner = self.inner.lock();
        let Some(inner) = inner.as_mut() else {
            return_errno_with_message!(Errno::ECONNREFUSED, "the target socket is closed");
        };
        if inner.is_shutdown {
            return_errno_with_message!(Errno::EPIPE, "the target socket is shut down");
        }

        let len = reader.sum_lens();
        if len > UNIX_DATAGRAM_DEFAULT_BUF_SIZE {
            return_errno_with_message!(Errno::EMSGSIZE, "the message is too large");
        }
        if UNIX_DATAGRAM_DEFAULT_BUF_SIZE - inner.total_length < len {
            return_errno_with_message!(
                Errno::EAGAIN,
                "the receive buffer does not have enough space"
            );
        }

        let msg = {
            let mut bytes = vec![0; len];
            reader.read(&mut VmWriter::from(bytes.as_mut_slice()))?;

            let mut aux = core::mem::take(aux_data);
            if self.is_pass_cred.load(Ordering::Relaxed)
                || source.queue.is_pass_cred.load(Ordering::Relaxed)
            {
                aux.fill_cred();
            }

            let src = source.queue.addr();

            Message { bytes, aux, src }
        };

        inner.total_length += msg.bytes.len();
        inner.messages.push_back(msg);

        self.pollee.notify(IoEvents::IN);

        Ok(len)
    }

    pub(super) fn addr(&self) -> UnixSocketAddr {
        self.addr.get().cloned().unwrap_or(UnixSocketAddr::Unnamed)
    }

    /// Blocks until the buffer is free and the `try_send` succeeds, or until interrupted.
    pub(super) fn block_send<F, R>(&self, mut try_send: F) -> Result<R>
    where
        F: FnMut() -> Result<R>,
    {
        self.send_wait_queue.pause_until(|| match try_send() {
            Err(err) if err.error() == Errno::EAGAIN => None,
            result => Some(result),
        })?
    }
}

// Note that a message receiver corresponds to a live socket and maintains certain invariants. For
// instance, `queue.inner` is always `Some(_)`, and the queue is in the global table if it is bound
// (i.e., `addr` is not `None`).
pub(super) struct MessageReceiver {
    // `addr` should be dropped as soon as the socket file is closed,
    // so it must not belong to `MessageQueue`.
    addr: SpinLock<Option<UnixSocketAddrBound>>,
    queue: Arc<MessageQueue>,
}

impl MessageReceiver {
    pub(super) fn new() -> MessageReceiver {
        let inner = Inner {
            messages: VecDeque::new(),
            total_length: 0,
            is_shutdown: false,
        };

        let queue = MessageQueue {
            addr: Once::new(),
            inner: Mutex::new(Some(inner)),
            pollee: Pollee::new(),
            send_wait_queue: WaitQueue::new(),
            is_pass_cred: AtomicBool::new(false),
        };

        Self {
            addr: SpinLock::new(None),
            queue: Arc::new(queue),
        }
    }

    pub(super) fn bind(&self, addr_to_bind: UnixSocketAddr) -> Result<()> {
        let mut addr = self.addr.lock();

        if addr.is_some() {
            return addr_to_bind.bind_unnamed();
        }

        let bound_addr = addr_to_bind.bind()?;
        QUEUE_TABLE.add_queue(bound_addr.to_key(), self.queue.clone());
        self.queue.addr.call_once(|| bound_addr.clone().into());
        *addr = Some(bound_addr);

        Ok(())
    }

    pub(super) fn try_recv(
        &self,
        writer: &mut dyn MultiWrite,
    ) -> Result<(usize, Vec<ControlMessage>, UnixSocketAddr)> {
        let mut inner = self.queue.inner.lock();
        let inner = inner.as_mut().unwrap();

        let Some(msg) = inner.messages.front() else {
            if !inner.is_shutdown {
                return_errno_with_message!(Errno::EAGAIN, "the receive buffer is empty");
            } else {
                return Ok((0, Vec::new(), UnixSocketAddr::Unnamed));
            }
        };

        let len = writer.write(&mut VmReader::from(msg.bytes.as_slice()))?;
        if len != msg.bytes.len() {
            warn!("setting MSG_TRUNC is not supported");
        }

        let mut msg = inner.messages.pop_front().unwrap();
        inner.total_length -= msg.bytes.len();

        let is_pass_cred = self.queue.is_pass_cred.load(Ordering::Relaxed);
        let ctrl_msgs = msg.aux.generate_control(is_pass_cred);

        self.queue.pollee.invalidate();
        // A writer may still fail if the free space is not enough.
        // So we have to wake up all the writers here.
        self.queue.send_wait_queue.wake_all();

        Ok((len, ctrl_msgs, msg.src))
    }

    pub(super) fn shutdown(&self) {
        let mut inner = self.queue.inner.lock();
        let inner = inner.as_mut().unwrap();

        inner.is_shutdown = true;
        self.queue.send_wait_queue.wake_all();

        // The caller will notify the pollee.
    }

    pub(super) fn set_pass_cred(&self, is_pass_cred: bool) {
        self.queue
            .is_pass_cred
            .store(is_pass_cred, Ordering::Relaxed);
    }

    pub(super) fn addr(&self) -> UnixSocketAddr {
        self.queue.addr()
    }

    pub(super) fn queue(&self) -> &Arc<MessageQueue> {
        &self.queue
    }

    pub(super) fn pollee(&self) -> &Pollee {
        &self.queue.pollee
    }

    pub(super) fn check_io_events(&self) -> IoEvents {
        let inner = self.queue.inner.lock();
        let inner = inner.as_ref().unwrap();

        if inner.is_shutdown {
            IoEvents::IN | IoEvents::RDHUP
        } else if !inner.messages.is_empty() {
            IoEvents::IN
        } else {
            IoEvents::empty()
        }
    }
}

impl Drop for MessageReceiver {
    fn drop(&mut self) {
        if let Some(addr) = self.addr.get_mut().as_mut() {
            QUEUE_TABLE.remove_queue(&addr.to_key());
        }

        *self.queue.inner.lock() = None;
        self.queue.send_wait_queue.wake_all();
    }
}

static QUEUE_TABLE: QueueTable = QueueTable::new();

struct QueueTable {
    message_queues: RwLock<BTreeMap<UnixSocketAddrKey, Arc<MessageQueue>>>,
}

impl QueueTable {
    pub(self) const fn new() -> Self {
        Self {
            message_queues: RwLock::new(BTreeMap::new()),
        }
    }

    pub(self) fn add_queue(&self, addr_key: UnixSocketAddrKey, queue: Arc<MessageQueue>) {
        let old_queue = self.message_queues.write().insert(addr_key, queue);
        debug_assert!(old_queue.is_none());
    }

    pub(self) fn get_queue(&self, addr_key: &UnixSocketAddrKey) -> Option<Arc<MessageQueue>> {
        self.message_queues.read().get(addr_key).cloned()
    }

    pub(self) fn remove_queue(&self, addr_key: &UnixSocketAddrKey) {
        let old_queue = self.message_queues.write().remove(addr_key);
        debug_assert!(old_queue.is_some());
    }
}

pub(in crate::net) const UNIX_DATAGRAM_DEFAULT_BUF_SIZE: usize = 65536;
