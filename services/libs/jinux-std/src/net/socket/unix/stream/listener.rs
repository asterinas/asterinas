use super::{connected::Connected, endpoint::Endpoint, UnixStreamSocket};
use crate::events::IoEvents;
use crate::fs::file_handle::FileLike;
use crate::fs::utils::{Dentry, Inode};
use crate::net::socket::unix::addr::{UnixSocketAddr, UnixSocketAddrBound};
use crate::net::socket::SocketAddr;
use crate::prelude::*;
use crate::process::signal::{Pollee, Poller};
use core::sync::atomic::{AtomicBool, Ordering};
use keyable_arc::KeyableWeak;

pub(super) struct Listener {
    addr: UnixSocketAddrBound,
    is_nonblocking: AtomicBool,
}

impl Listener {
    pub(super) fn new(
        addr: UnixSocketAddrBound,
        backlog: usize,
        nonblocking: bool,
    ) -> Result<Self> {
        BACKLOG_TABLE.add_backlog(&addr, backlog)?;
        Ok(Self {
            addr,
            is_nonblocking: AtomicBool::new(nonblocking),
        })
    }

    pub(super) fn addr(&self) -> &UnixSocketAddrBound {
        &self.addr
    }

    pub(super) fn is_nonblocking(&self) -> bool {
        self.is_nonblocking.load(Ordering::Acquire)
    }

    pub(super) fn set_nonblocking(&self, is_nonblocking: bool) {
        self.is_nonblocking.store(is_nonblocking, Ordering::Release);
    }

    pub(super) fn accept(&self) -> Result<(Arc<dyn FileLike>, SocketAddr)> {
        let addr = self.addr().clone();
        let is_nonblocking = self.is_nonblocking();

        let connected = {
            let local_endpoint = BACKLOG_TABLE.pop_incoming(is_nonblocking, &addr)?;
            Connected::new(local_endpoint)
        };

        let peer_addr = match connected.peer_addr() {
            None => SocketAddr::Unix(UnixSocketAddr::Path(String::new())),
            Some(addr) => SocketAddr::from(addr.clone()),
        };

        let socket = Arc::new(UnixStreamSocket::new_connected(connected));

        Ok((socket, peer_addr))
    }

    pub(super) fn poll(&self, mask: IoEvents, poller: Option<&Poller>) -> IoEvents {
        let addr = self.addr();
        let backlog = BACKLOG_TABLE.get_backlog(addr).unwrap();
        backlog.poll(mask, poller)
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

    fn add_backlog(&self, addr: &UnixSocketAddrBound, backlog: usize) -> Result<()> {
        let inode = {
            let UnixSocketAddrBound::Path(dentry) = addr else {
                todo!()
            };
            create_keyable_inode(dentry)
        };

        let mut backlog_sockets = self.backlog_sockets.write();
        if backlog_sockets.contains_key(&inode) {
            return_errno_with_message!(Errno::EADDRINUSE, "the addr is already used");
        }
        let new_backlog = Arc::new(Backlog::new(backlog));
        backlog_sockets.insert(inode, new_backlog);
        Ok(())
    }

    fn get_backlog(&self, addr: &UnixSocketAddrBound) -> Result<Arc<Backlog>> {
        let inode = {
            let UnixSocketAddrBound::Path(dentry) = addr else {
                todo!()
            };
            create_keyable_inode(dentry)
        };

        let backlog_sockets = self.backlog_sockets.read();
        backlog_sockets
            .get(&inode)
            .map(Arc::clone)
            .ok_or_else(|| Error::with_message(Errno::EINVAL, "the socket is not listened"))
    }

    fn pop_incoming(&self, nonblocking: bool, addr: &UnixSocketAddrBound) -> Result<Arc<Endpoint>> {
        let poller = Poller::new();
        loop {
            let backlog = self.get_backlog(addr)?;

            if let Some(endpoint) = backlog.pop_incoming() {
                return Ok(endpoint);
            }

            if nonblocking {
                return_errno_with_message!(Errno::EAGAIN, "no connection comes");
            }

            let events = {
                let mask = IoEvents::IN;
                backlog.poll(mask, Some(&poller))
            };

            if events.contains(IoEvents::ERR) | events.contains(IoEvents::HUP) {
                return_errno_with_message!(Errno::ECONNABORTED, "connection is aborted");
            }

            // FIXME: deal with accept timeout
            if events.is_empty() {
                poller.wait()?;
            }
        }
    }

    fn push_incoming(&self, addr: &UnixSocketAddrBound, endpoint: Arc<Endpoint>) -> Result<()> {
        let backlog = self.get_backlog(addr).map_err(|_| {
            Error::with_message(
                Errno::ECONNREFUSED,
                "no socket is listened at the remote address",
            )
        })?;

        backlog.push_incoming(endpoint)
    }

    fn remove_backlog(&self, addr: &UnixSocketAddrBound) {
        let UnixSocketAddrBound::Path(dentry) = addr else {
            todo!()
        };

        let inode = create_keyable_inode(dentry);
        self.backlog_sockets.write().remove(&inode);
    }
}

struct Backlog {
    pollee: Pollee,
    backlog: usize,
    incoming_endpoints: Mutex<VecDeque<Arc<Endpoint>>>,
}

impl Backlog {
    fn new(backlog: usize) -> Self {
        Self {
            pollee: Pollee::new(IoEvents::empty()),
            backlog,
            incoming_endpoints: Mutex::new(VecDeque::with_capacity(backlog)),
        }
    }

    fn push_incoming(&self, endpoint: Arc<Endpoint>) -> Result<()> {
        let mut endpoints = self.incoming_endpoints.lock();
        if endpoints.len() >= self.backlog {
            return_errno_with_message!(Errno::ECONNREFUSED, "incoming_endpoints is full");
        }
        endpoints.push_back(endpoint);
        self.pollee.add_events(IoEvents::IN);
        Ok(())
    }

    fn pop_incoming(&self) -> Option<Arc<Endpoint>> {
        let mut incoming_endpoints = self.incoming_endpoints.lock();
        let endpoint = incoming_endpoints.pop_front();
        if incoming_endpoints.is_empty() {
            self.pollee.del_events(IoEvents::IN);
        }
        endpoint
    }

    fn poll(&self, mask: IoEvents, poller: Option<&Poller>) -> IoEvents {
        // Lock to avoid any events may change pollee state when we poll
        let _lock = self.incoming_endpoints.lock();
        self.pollee.poll(mask, poller)
    }
}

fn create_keyable_inode(dentry: &Arc<Dentry>) -> KeyableWeak<dyn Inode> {
    let weak_inode = Arc::downgrade(dentry.inode());
    KeyableWeak::from(weak_inode)
}

pub(super) fn unregister_backlog(addr: &UnixSocketAddrBound) {
    BACKLOG_TABLE.remove_backlog(addr);
}

pub(super) fn push_incoming(
    remote_addr: &UnixSocketAddrBound,
    remote_end: Arc<Endpoint>,
) -> Result<()> {
    BACKLOG_TABLE.push_incoming(remote_addr, remote_end)
}
