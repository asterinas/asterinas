// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicUsize, Ordering};

use ostd::sync::WaitQueue;

use super::{
    connected::{combine_io_events, Connected},
    init::Init,
    UnixStreamSocket,
};
use crate::{
    events::{IoEvents, Observer},
    fs::file_handle::FileLike,
    net::socket::{
        unix::addr::{UnixSocketAddrBound, UnixSocketAddrKey},
        SockShutdownCmd, SocketAddr,
    },
    prelude::*,
    process::signal::{Pollee, Poller},
};

pub(super) struct Listener {
    backlog: Arc<Backlog>,
    writer_pollee: Pollee,
}

impl Listener {
    pub(super) fn new(
        addr: UnixSocketAddrBound,
        reader_pollee: Pollee,
        writer_pollee: Pollee,
        backlog: usize,
        is_shutdown: bool,
    ) -> Self {
        // Note that the I/O events can be correctly inherited from `Init`. There is no need to
        // explicitly call `Pollee::reset_io_events`.
        let backlog = BACKLOG_TABLE
            .add_backlog(addr, reader_pollee, backlog, is_shutdown)
            .unwrap();
        writer_pollee.del_events(IoEvents::OUT);

        Self {
            backlog,
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
                self.writer_pollee.add_events(IoEvents::ERR);
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

    pub(super) fn poll(&self, mask: IoEvents, mut poller: Option<&mut Poller>) -> IoEvents {
        let reader_events = self.backlog.poll(mask, poller.as_deref_mut());
        let writer_events = self.writer_pollee.poll(mask, poller);

        combine_io_events(mask, reader_events, writer_events)
    }

    pub(super) fn register_observer(
        &self,
        observer: Weak<dyn Observer<IoEvents>>,
        mask: IoEvents,
    ) -> Result<()> {
        self.backlog.register_observer(observer.clone(), mask)?;
        self.writer_pollee.register_observer(observer, mask);
        Ok(())
    }

    pub(super) fn unregister_observer(
        &self,
        observer: &Weak<dyn Observer<IoEvents>>,
    ) -> Option<Weak<dyn Observer<IoEvents>>> {
        let reader_observer = self.backlog.unregister_observer(observer);
        let writer_observer = self.writer_pollee.unregister_observer(observer);
        reader_observer.or(writer_observer)
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
        let conn = self.incoming_conns.lock_with(|locked_incoming_conns| {
            let Some(incoming_conns) = &mut *locked_incoming_conns else {
                return_errno_with_message!(Errno::EINVAL, "the socket is shut down for reading");
            };

            let conn = incoming_conns.pop_front();
            if incoming_conns.is_empty() {
                self.pollee.del_events(IoEvents::IN);
            }
            Result::Ok(conn)
        })?;

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
        self.incoming_conns.lock_with(|incoming_conns| {
            *incoming_conns = None;
            self.pollee.add_events(IoEvents::HUP);
            self.pollee.del_events(IoEvents::IN);
        });

        self.wait_queue.wake_all();
    }

    fn poll(&self, mask: IoEvents, poller: Option<&mut Poller>) -> IoEvents {
        self.pollee.poll(mask, poller)
    }

    fn register_observer(
        &self,
        observer: Weak<dyn Observer<IoEvents>>,
        mask: IoEvents,
    ) -> Result<()> {
        self.pollee.register_observer(observer, mask);
        Ok(())
    }

    fn unregister_observer(
        &self,
        observer: &Weak<dyn Observer<IoEvents>>,
    ) -> Option<Weak<dyn Observer<IoEvents>>> {
        self.pollee.unregister_observer(observer)
    }
}

impl Backlog {
    pub(super) fn push_incoming(
        &self,
        init: Init,
    ) -> core::result::Result<Connected, (Error, Init)> {
        self.incoming_conns.lock_with(|locked_incoming_conns| {
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

            self.pollee.add_events(IoEvents::IN);

            Ok(client_conn)
        })
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
