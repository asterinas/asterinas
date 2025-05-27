// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use aster_rights::ReadDupOp;
use ostd::sync::WaitQueue;

use super::{
    connected::Connected,
    init::Init,
    socket::{SHUT_READ_EVENTS, SHUT_WRITE_EVENTS},
    UnixStreamSocket,
};
use crate::{
    events::IoEvents,
    fs::file_handle::FileLike,
    net::socket::{
        unix::{
            addr::{UnixSocketAddrBound, UnixSocketAddrKey},
            cred::SocketCred,
            stream::socket::OptionSet,
        },
        util::{options::SocketOptionSet, SockShutdownCmd, SocketAddr},
    },
    prelude::*,
    process::signal::Pollee,
};

pub(super) struct Listener {
    backlog: Arc<Backlog>,
    is_write_shutdown: AtomicBool,
}

impl Listener {
    pub(super) fn new(
        addr: UnixSocketAddrBound,
        backlog: usize,
        is_read_shutdown: bool,
        is_write_shutdown: bool,
        pollee: Pollee,
        is_seqpacket: bool,
    ) -> Self {
        let backlog = BACKLOG_TABLE
            .add_backlog(addr, pollee, backlog, is_read_shutdown, is_seqpacket)
            .unwrap();

        Self {
            backlog,
            is_write_shutdown: AtomicBool::new(is_write_shutdown),
        }
    }

    pub(super) fn addr(&self) -> &UnixSocketAddrBound {
        self.backlog.addr()
    }

    pub(super) fn try_accept(&self, is_seqpacket: bool) -> Result<(Arc<dyn FileLike>, SocketAddr)> {
        let connected = self.backlog.pop_incoming()?;

        let peer_addr = connected.peer_addr().into();
        // TODO: Update options for a newly-accepted socket
        let options = OptionSet::new();
        let socket = UnixStreamSocket::new_connected(connected, options, false, is_seqpacket);

        Ok((socket, peer_addr))
    }

    pub(super) fn listen(&self, backlog: usize) {
        self.backlog.set_backlog(backlog);
    }

    pub(super) fn shutdown(&self, cmd: SockShutdownCmd, pollee: &Pollee) {
        if cmd.shut_read() {
            self.backlog.shutdown();
        }

        if cmd.shut_write() {
            self.is_write_shutdown.store(true, Ordering::Relaxed);
            pollee.notify(SHUT_WRITE_EVENTS);
        }
    }

    pub(super) fn is_read_shutdown(&self) -> bool {
        self.backlog.is_shutdown()
    }

    pub(super) fn is_write_shutdown(&self) -> bool {
        self.is_write_shutdown.load(Ordering::Relaxed)
    }

    pub(super) fn check_io_events(&self) -> IoEvents {
        self.backlog.check_io_events()
    }

    pub(super) fn cred(&self) -> &SocketCred<ReadDupOp> {
        &self.backlog.listener_cred
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
        is_seqpacket: bool,
    ) -> Option<Arc<Backlog>> {
        let addr_key = addr.to_key();

        let mut backlog_sockets = self.backlog_sockets.write();

        if backlog_sockets.contains_key(&addr_key) {
            return None;
        }

        let new_backlog = Arc::new(Backlog::new(
            addr,
            pollee,
            backlog,
            is_shutdown,
            is_seqpacket,
        ));
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
    listener_cred: SocketCred<ReadDupOp>,
    is_seqpacket: bool,
}

impl Backlog {
    fn new(
        addr: UnixSocketAddrBound,
        pollee: Pollee,
        backlog: usize,
        is_shutdown: bool,
        is_seqpacket: bool,
    ) -> Self {
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
            listener_cred: SocketCred::<ReadDupOp>::new_current(),
            is_seqpacket,
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

        self.pollee.invalidate();

        drop(locked_incoming_conns);

        if conn.is_some() {
            self.pollee.invalidate();
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
        *self.incoming_conns.lock() = None;

        self.pollee.notify(SHUT_READ_EVENTS);
        self.wait_queue.wake_all();
    }

    fn is_shutdown(&self) -> bool {
        self.incoming_conns.lock().is_none()
    }

    fn check_io_events(&self) -> IoEvents {
        if self
            .incoming_conns
            .lock()
            .as_ref()
            .is_some_and(|conns| !conns.is_empty())
        {
            IoEvents::IN
        } else {
            IoEvents::empty()
        }
    }
}

impl Backlog {
    pub(super) fn push_incoming(
        &self,
        init: Init,
        pollee: Pollee,
        options: &SocketOptionSet,
        is_seqpacket: bool,
    ) -> core::result::Result<Connected, (Error, Init)> {
        if is_seqpacket != self.is_seqpacket {
            // FIXME: According to the Linux implementation, we should avoid this error by
            // maintaining two socket tables for SOCK_STREAM sockets and SOCK_SEQPACKET sockets
            // separately.
            return Err((
                Error::with_message(
                    Errno::ECONNREFUSED,
                    "the listening socket has a different socket type",
                ),
                init,
            ));
        }

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

        let (client_conn, server_conn) = init.into_connected(
            self.addr.clone(),
            pollee,
            self.listener_cred.dup().restrict(),
            options,
        );

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
