// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use ostd::sync::WaitQueue;

use super::{
    connected::{combine_io_events, Connected},
    init::Init,
    UnixStreamSocket,
};
use crate::{
    events::IoEvents,
    fs::file_handle::FileLike,
    net::socket::{
        unix::addr::{UnixSocketAddrBound, UnixSocketAddrKey},
        SockShutdownCmd, SocketAddr,
    },
    prelude::*,
    process::signal::{PollHandle, Pollee},
};

pub(super) struct Listener {
    backlog: Arc<Backlog>,
    is_write_shutdown: AtomicBool,
    writer_pollee: Pollee,
}

impl Listener {
    pub(super) fn new(
        addr: UnixSocketAddrBound,
        reader_pollee: Pollee,
        writer_pollee: Pollee,
        backlog: usize,
        is_read_shutdown: bool,
        is_write_shutdown: bool,
    ) -> Self {
        let backlog = BACKLOG_TABLE
            .add_backlog(addr, reader_pollee, backlog, is_read_shutdown)
            .unwrap();
        writer_pollee.invalidate();

        Self {
            backlog,
            is_write_shutdown: AtomicBool::new(is_write_shutdown),
            writer_pollee,
        }
    }

    pub(super) fn addr(&self) -> &UnixSocketAddrBound {
        self.backlog.addr()
    }

    pub(super) fn try_accept(&self) -> Result<(Arc<dyn FileLike>, SocketAddr)> {
        let connected = self.backlog.pop_incoming()?;
        let peer_addr = connected.peer_addr().into();

        let socket = UnixStreamSocket::new_connected(connected, false);
        Ok((socket, peer_addr))
    }

    pub(super) fn listen(&self, backlog: usize) {
        self.backlog.set_backlog(backlog);
    }

    pub(super) fn shutdown(&self, cmd: SockShutdownCmd) {
        match cmd {
            SockShutdownCmd::SHUT_WR | SockShutdownCmd::SHUT_RDWR => {
                self.is_write_shutdown.store(true, Ordering::Relaxed);
                self.writer_pollee.notify(IoEvents::ERR);
            }
            SockShutdownCmd::SHUT_RD => (),
        }

        match cmd {
            SockShutdownCmd::SHUT_RD | SockShutdownCmd::SHUT_RDWR => {
                self.backlog.shutdown();
            }
            SockShutdownCmd::SHUT_WR => (),
        }
    }

    pub(super) fn poll(&self, mask: IoEvents, mut poller: Option<&mut PollHandle>) -> IoEvents {
        let reader_events = self.backlog.poll(mask, poller.as_deref_mut());

        let writer_events = self.writer_pollee.poll_with(mask, poller, || {
            if self.is_write_shutdown.load(Ordering::Relaxed) {
                IoEvents::ERR
            } else {
                IoEvents::empty()
            }
        });

        combine_io_events(mask, reader_events, writer_events)
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        self.backlog.shutdown();

        unregister_backlog(&self.backlog.addr().to_key())
    }
}

static BACKLOG_TABLE: BacklogTable = BacklogTable::new();

struct BacklogTable {
    backlog_sockets: RwLock<BTreeMap<UnixSocketAddrKey, Arc<Backlog>>>,
}

impl BacklogTable {
    const fn new() -> Self {
        Self {
            backlog_sockets: RwLock::new(BTreeMap::new()),
        }
    }

    fn add_backlog(
        &self,
        addr: UnixSocketAddrBound,
        pollee: Pollee,
        backlog: usize,
        is_shutdown: bool,
    ) -> Option<Arc<Backlog>> {
        let addr_key = addr.to_key();

        let mut backlog_sockets = self.backlog_sockets.write();

        if backlog_sockets.contains_key(&addr_key) {
            return None;
        }

        // Note that the cached events can be correctly inherited from `Init`, so there is no need
        // to explicitly call `Pollee::invalidate`.
        let new_backlog = Arc::new(Backlog::new(addr, pollee, backlog, is_shutdown));
        backlog_sockets.insert(addr_key, new_backlog.clone());

        Some(new_backlog)
    }

    fn get_backlog(&self, addr: &UnixSocketAddrKey) -> Option<Arc<Backlog>> {
        self.backlog_sockets.read().get(addr).cloned()
    }

    fn remove_backlog(&self, addr_key: &UnixSocketAddrKey) {
        self.backlog_sockets.write().remove(addr_key);
    }
}

pub(super) struct Backlog {
    addr: UnixSocketAddrBound,
    pollee: Pollee,
    backlog: AtomicUsize,
    incoming_conns: SpinLock<Option<VecDeque<Connected>>>,
    wait_queue: WaitQueue,
}

impl Backlog {
    fn new(addr: UnixSocketAddrBound, pollee: Pollee, backlog: usize, is_shutdown: bool) -> Self {
        let incoming_sockets = if is_shutdown {
            None
        } else {
            Some(VecDeque::with_capacity(backlog))
        };

        Self {
            addr,
            pollee,
            backlog: AtomicUsize::new(backlog),
            incoming_conns: SpinLock::new(incoming_sockets),
            wait_queue: WaitQueue::new(),
        }
    }

    fn addr(&self) -> &UnixSocketAddrBound {
        &self.addr
    }

    fn pop_incoming(&self) -> Result<Connected> {
        let mut locked_incoming_conns = self.incoming_conns.lock();

        let Some(incoming_conns) = &mut *locked_incoming_conns else {
            return_errno_with_message!(Errno::EINVAL, "the socket is shut down for reading");
        };
        let conn = incoming_conns.pop_front();

        drop(locked_incoming_conns);

        if conn.is_some() {
            self.wait_queue.wake_one();
        }

        conn.ok_or_else(|| Error::with_message(Errno::EAGAIN, "no pending connection is available"))
    }

    fn set_backlog(&self, backlog: usize) {
        let old_backlog = self.backlog.swap(backlog, Ordering::Relaxed);

        if old_backlog < backlog {
            self.wait_queue.wake_all();
        }
    }

    fn shutdown(&self) {
        let mut incoming_conns = self.incoming_conns.lock();

        *incoming_conns = None;
        self.pollee.notify(IoEvents::HUP);

        drop(incoming_conns);

        self.wait_queue.wake_all();
    }

    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        self.pollee
            .poll_with(mask, poller, || self.check_io_events())
    }

    fn check_io_events(&self) -> IoEvents {
        let incoming_conns = self.incoming_conns.lock();

        if let Some(conns) = &*incoming_conns {
            if !conns.is_empty() {
                IoEvents::IN
            } else {
                IoEvents::empty()
            }
        } else {
            IoEvents::HUP
        }
    }
}

impl Backlog {
    pub(super) fn push_incoming(
        &self,
        init: Init,
    ) -> core::result::Result<Connected, (Error, Init)> {
        let mut locked_incoming_conns = self.incoming_conns.lock();

        let Some(incoming_conns) = &mut *locked_incoming_conns else {
            return Err((
                Error::with_message(
                    Errno::ECONNREFUSED,
                    "the listening socket is shut down for reading",
                ),
                init,
            ));
        };

        if incoming_conns.len() >= self.backlog.load(Ordering::Relaxed) {
            return Err((
                Error::with_message(
                    Errno::EAGAIN,
                    "the pending connection queue on the listening socket is full",
                ),
                init,
            ));
        }

        let (client_conn, server_conn) = init.into_connected(self.addr.clone());

        incoming_conns.push_back(server_conn);
        self.pollee.notify(IoEvents::IN);

        Ok(client_conn)
    }

    pub(super) fn pause_until<F>(&self, mut cond: F) -> Result<()>
    where
        F: FnMut() -> Result<()>,
    {
        self.wait_queue.pause_until(|| match cond() {
            Err(err) if err.error() == Errno::EAGAIN => None,
            result => Some(result),
        })?
    }
}

fn unregister_backlog(addr: &UnixSocketAddrKey) {
    BACKLOG_TABLE.remove_backlog(addr);
}

pub(super) fn get_backlog(server_key: &UnixSocketAddrKey) -> Result<Arc<Backlog>> {
    BACKLOG_TABLE.get_backlog(server_key).ok_or_else(|| {
        Error::with_message(
            Errno::ECONNREFUSED,
            "no socket is listening at the remote address",
        )
    })
}
