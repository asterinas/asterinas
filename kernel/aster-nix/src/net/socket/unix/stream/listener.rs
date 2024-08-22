// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicUsize, Ordering};

use keyable_arc::KeyableWeak;

use super::{connected::Connected, UnixStreamSocket};
use crate::{
    events::{IoEvents, Observer},
    fs::{file_handle::FileLike, path::Dentry, utils::Inode},
    net::socket::{unix::addr::UnixSocketAddrBound, SocketAddr},
    prelude::*,
    process::signal::{Pollee, Poller},
};

pub(super) struct Listener {
    backlog: Arc<Backlog>,
}

impl Listener {
    pub(super) fn new(addr: UnixSocketAddrBound, backlog: usize) -> Self {
        let backlog = BACKLOG_TABLE.add_backlog(addr, backlog).unwrap();
        Self { backlog }
    }

    pub(super) fn addr(&self) -> &UnixSocketAddrBound {
        self.backlog.addr()
    }

    pub(super) fn try_accept(&self) -> Result<(Arc<dyn FileLike>, SocketAddr)> {
        let connected = self.backlog.pop_incoming()?;
        let peer_addr = connected.peer_addr().cloned().into();

        let socket = UnixStreamSocket::new_connected(connected, false);
        Ok((socket, peer_addr))
    }

    pub(super) fn listen(&self, backlog: usize) -> Result<()> {
        self.backlog.set_backlog(backlog);
        Ok(())
    }

    pub(super) fn poll(&self, mask: IoEvents, poller: Option<&mut Poller>) -> IoEvents {
        self.backlog.poll(mask, poller)
    }

    pub(super) fn register_observer(
        &self,
        observer: Weak<dyn Observer<IoEvents>>,
        mask: IoEvents,
    ) -> Result<()> {
        self.backlog.register_observer(observer, mask)
    }

    pub(super) fn unregister_observer(
        &self,
        observer: &Weak<dyn Observer<IoEvents>>,
    ) -> Option<Weak<dyn Observer<IoEvents>>> {
        self.backlog.unregister_observer(observer)
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        unregister_backlog(self.backlog.addr())
    }
}

static BACKLOG_TABLE: BacklogTable = BacklogTable::new();

struct BacklogTable {
    backlog_sockets: RwLock<BTreeMap<KeyableWeak<dyn Inode>, Arc<Backlog>>>,
    // TODO: For linux, there is also abstract socket domain that a socket addr is not bound to an inode.
}

impl BacklogTable {
    const fn new() -> Self {
        Self {
            backlog_sockets: RwLock::new(BTreeMap::new()),
        }
    }

    fn add_backlog(&self, addr: UnixSocketAddrBound, backlog: usize) -> Option<Arc<Backlog>> {
        let inode = {
            let UnixSocketAddrBound::Path(_, ref dentry) = addr else {
                todo!()
            };
            create_keyable_inode(dentry)
        };
        let new_backlog = Arc::new(Backlog::new(addr, backlog));

        let mut backlog_sockets = self.backlog_sockets.write();
        if backlog_sockets.contains_key(&inode) {
            return None;
        }
        backlog_sockets.insert(inode, new_backlog.clone());

        Some(new_backlog)
    }

    fn get_backlog(&self, addr: &UnixSocketAddrBound) -> Option<Arc<Backlog>> {
        let inode = {
            let UnixSocketAddrBound::Path(_, dentry) = addr else {
                todo!()
            };
            create_keyable_inode(dentry)
        };

        let backlog_sockets = self.backlog_sockets.read();
        backlog_sockets.get(&inode).cloned()
    }

    fn push_incoming(
        &self,
        server_addr: &UnixSocketAddrBound,
        client_addr: Option<UnixSocketAddrBound>,
    ) -> Result<Connected> {
        let backlog = self.get_backlog(server_addr).ok_or_else(|| {
            Error::with_message(
                Errno::ECONNREFUSED,
                "no socket is listening at the remote address",
            )
        })?;

        backlog.push_incoming(client_addr)
    }

    fn remove_backlog(&self, addr: &UnixSocketAddrBound) {
        let UnixSocketAddrBound::Path(_, dentry) = addr else {
            todo!()
        };

        let inode = create_keyable_inode(dentry);
        self.backlog_sockets.write().remove(&inode);
    }
}

struct Backlog {
    addr: UnixSocketAddrBound,
    pollee: Pollee,
    backlog: AtomicUsize,
    incoming_conns: Mutex<VecDeque<Connected>>,
}

impl Backlog {
    fn new(addr: UnixSocketAddrBound, backlog: usize) -> Self {
        Self {
            addr,
            pollee: Pollee::new(IoEvents::empty()),
            backlog: AtomicUsize::new(backlog),
            incoming_conns: Mutex::new(VecDeque::with_capacity(backlog)),
        }
    }

    fn addr(&self) -> &UnixSocketAddrBound {
        &self.addr
    }

    fn push_incoming(&self, client_addr: Option<UnixSocketAddrBound>) -> Result<Connected> {
        let mut incoming_conns = self.incoming_conns.lock();

        if incoming_conns.len() >= self.backlog.load(Ordering::Relaxed) {
            return_errno_with_message!(
                Errno::EAGAIN,
                "the pending connection queue on the listening socket is full"
            );
        }

        let (server_conn, client_conn) = Connected::new_pair(Some(self.addr.clone()), client_addr);
        incoming_conns.push_back(server_conn);

        self.pollee.add_events(IoEvents::IN);

        Ok(client_conn)
    }

    fn pop_incoming(&self) -> Result<Connected> {
        let mut incoming_conns = self.incoming_conns.lock();
        let conn = incoming_conns.pop_front();
        if incoming_conns.is_empty() {
            self.pollee.del_events(IoEvents::IN);
        }
        conn.ok_or_else(|| Error::with_message(Errno::EAGAIN, "no pending connection is available"))
    }

    fn set_backlog(&self, backlog: usize) {
        self.backlog.store(backlog, Ordering::Relaxed);
    }

    fn poll(&self, mask: IoEvents, poller: Option<&mut Poller>) -> IoEvents {
        // Lock to avoid any events may change pollee state when we poll
        let _lock = self.incoming_conns.lock();
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

fn create_keyable_inode(dentry: &Arc<Dentry>) -> KeyableWeak<dyn Inode> {
    let weak_inode = Arc::downgrade(dentry.inode());
    KeyableWeak::from(weak_inode)
}

fn unregister_backlog(addr: &UnixSocketAddrBound) {
    BACKLOG_TABLE.remove_backlog(addr);
}

pub(super) fn push_incoming(
    server_addr: &UnixSocketAddrBound,
    client_addr: Option<UnixSocketAddrBound>,
) -> Result<Connected> {
    BACKLOG_TABLE.push_incoming(server_addr, client_addr)
}
