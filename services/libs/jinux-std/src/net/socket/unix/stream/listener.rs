use keyable_arc::KeyableWeak;
use spin::RwLockReadGuard;

use crate::{
    fs::utils::{Inode, IoEvents, Pollee, Poller},
    net::socket::unix::addr::UnixSocketAddr,
    prelude::*,
};

use super::endpoint::Endpoint;

pub static ACTIVE_LISTENERS: ActiveListeners = ActiveListeners::new();

pub struct ActiveListeners {
    listeners: RwLock<BTreeMap<KeyableWeak<dyn Inode>, Arc<Listener>>>,
    // TODO: For linux, there is also abstract socket domain that a socket addr is not bound to an inode.
}

impl ActiveListeners {
    pub const fn new() -> Self {
        Self {
            listeners: RwLock::new(BTreeMap::new()),
        }
    }

    pub(super) fn add_listener(&self, addr: &UnixSocketAddr, backlog: usize) -> Result<()> {
        let inode = create_keyable_inode(addr)?;
        let mut listeners = self.listeners.write();
        if listeners.contains_key(&inode) {
            return_errno_with_message!(Errno::EADDRINUSE, "the addr is already used");
        }
        let new_listener = Arc::new(Listener::new(backlog));
        listeners.insert(inode, new_listener);
        Ok(())
    }

    pub(super) fn get_listener(&self, addr: &UnixSocketAddr) -> Result<Arc<Listener>> {
        let listeners = self.listeners.read();
        get_listener(&listeners, addr)
    }

    pub(super) fn pop_incoming(
        &self,
        nonblocking: bool,
        addr: &UnixSocketAddr,
    ) -> Result<Arc<Endpoint>> {
        let poller = Poller::new();
        loop {
            let listener = {
                let listeners = self.listeners.read();
                get_listener(&listeners, addr)?
            };
            if let Some(endpoint) = listener.pop_incoming() {
                return Ok(endpoint);
            }
            if nonblocking {
                return_errno_with_message!(Errno::EAGAIN, "no connection comes");
            }
            let events = {
                let mask = IoEvents::IN;
                listener.poll(mask, Some(&poller))
            };
            if events.contains(IoEvents::ERR) | events.contains(IoEvents::HUP) {
                return_errno_with_message!(Errno::EINVAL, "connection is refused");
            }
            if events.is_empty() {
                poller.wait();
            }
        }
    }

    pub(super) fn push_incoming(
        &self,
        addr: &UnixSocketAddr,
        endpoint: Arc<Endpoint>,
    ) -> Result<()> {
        let listeners = self.listeners.read();
        let listener = get_listener(&listeners, addr).map_err(|_| {
            Error::with_message(
                Errno::ECONNREFUSED,
                "no socket is listened at the remote address",
            )
        })?;
        listener.push_incoming(endpoint)
    }

    pub(super) fn remove_listener(&self, addr: &UnixSocketAddr) {
        let Ok(inode) = create_keyable_inode(addr) else {
            return;
        };
        self.listeners.write().remove(&inode);
    }
}

fn get_listener(
    listeners: &RwLockReadGuard<BTreeMap<KeyableWeak<dyn Inode>, Arc<Listener>>>,
    addr: &UnixSocketAddr,
) -> Result<Arc<Listener>> {
    let dentry = create_keyable_inode(addr)?;
    listeners
        .get(&dentry)
        .map(Arc::clone)
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "the socket is not listened"))
}

pub(super) struct Listener {
    pollee: Pollee,
    backlog: usize,
    incoming_endpoints: Mutex<VecDeque<Arc<Endpoint>>>,
}

impl Listener {
    pub fn new(backlog: usize) -> Self {
        Self {
            pollee: Pollee::new(IoEvents::empty()),
            backlog,
            incoming_endpoints: Mutex::new(VecDeque::with_capacity(backlog)),
        }
    }

    pub fn push_incoming(&self, endpoint: Arc<Endpoint>) -> Result<()> {
        let mut endpoints = self.incoming_endpoints.lock();
        if endpoints.len() >= self.backlog {
            return_errno_with_message!(Errno::ECONNREFUSED, "incoming_endpoints is full");
        }
        endpoints.push_back(endpoint);
        self.pollee.add_events(IoEvents::IN);
        Ok(())
    }

    pub fn pop_incoming(&self) -> Option<Arc<Endpoint>> {
        let mut incoming_endpoints = self.incoming_endpoints.lock();
        let endpoint = incoming_endpoints.pop_front();
        if endpoint.is_none() {
            self.pollee.del_events(IoEvents::IN);
        }
        endpoint
    }

    pub fn poll(&self, mask: IoEvents, poller: Option<&Poller>) -> IoEvents {
        // Lock to avoid any events may change pollee state when we poll
        let _lock = self.incoming_endpoints.lock();
        self.pollee.poll(mask, poller)
    }
}

fn create_keyable_inode(addr: &UnixSocketAddr) -> Result<KeyableWeak<dyn Inode>> {
    let dentry = addr.dentry()?;
    let inode = dentry.inode();
    Ok(KeyableWeak::from(inode))
}
