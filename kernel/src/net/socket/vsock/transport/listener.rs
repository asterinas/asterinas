// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicUsize, Ordering};

use aster_softirq::BottomHalfDisabled;
use ostd::sync::SpinLock;

use crate::{
    events::IoEvents,
    net::socket::vsock::{
        VsockSocketAddr,
        transport::{BoundPort, Connection, MAX_BACKLOG, connection::ConnectionInner},
    },
    prelude::*,
    process::signal::Pollee,
};

/// A uniquely owned vsock listener handle; dropping it will close the listener.
pub(in crate::net::socket::vsock) struct Listener {
    inner: Arc<ListenerInner>,
}

impl Listener {
    pub(super) fn new(inner: Arc<ListenerInner>) -> Self {
        Self { inner }
    }

    /// Accepts one pending connection.
    pub(in crate::net::socket::vsock) fn try_accept(&self) -> Result<Connection> {
        self.inner.pop_incoming().map(Connection::new)
    }

    /// Updates the listen backlog.
    pub(in crate::net::socket::vsock) fn set_backlog(&self, backlog: usize) {
        self.inner.set_backlog(backlog);
    }

    /// Returns the local listening address.
    pub(in crate::net::socket::vsock) fn local_addr(&self) -> VsockSocketAddr {
        self.inner.bound_port.local_addr()
    }

    /// Returns the currently observable I/O readiness for the listener.
    pub(in crate::net::socket::vsock) fn check_io_events(&self) -> IoEvents {
        self.inner.check_io_events()
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        let vsock_space = self.inner.bound_port.vsock_space();
        vsock_space.remove_listener(&self.inner);
    }
}

pub(super) struct ListenerInner {
    bound_port: BoundPort,
    pollee: Pollee,
    backlog: AtomicUsize,
    num_conns: AtomicUsize,
    incoming_conns: SpinLock<VecDeque<Arc<ConnectionInner>>, BottomHalfDisabled>,
}

impl ListenerInner {
    pub(super) fn new(bound_port: BoundPort, backlog: usize, pollee: Pollee) -> Arc<Self> {
        pollee.invalidate();

        Arc::new(Self {
            bound_port,
            pollee,
            backlog: AtomicUsize::new(backlog.min(MAX_BACKLOG)),
            num_conns: AtomicUsize::new(0),
            incoming_conns: SpinLock::new(VecDeque::new()),
        })
    }

    pub(super) fn is_full(&self) -> bool {
        // Race conditions don't matter here. We use `>` instead of `>=` because Linux allows to
        // have `backlog + 1` connections in the backlog queue.
        self.num_conns.load(Ordering::Relaxed) > self.backlog.load(Ordering::Relaxed)
    }

    pub(super) fn push_incoming(&self, connection: Arc<ConnectionInner>) {
        let mut incoming_conns = self.incoming_conns.lock();
        incoming_conns.push_back(connection);
        self.num_conns
            .store(incoming_conns.len(), Ordering::Relaxed);

        drop(incoming_conns);
        self.pollee.notify(IoEvents::IN);
    }

    pub(self) fn pop_incoming(&self) -> Result<Arc<ConnectionInner>> {
        let mut incoming_conns = self.incoming_conns.lock();
        let Some(connection) = incoming_conns.pop_front() else {
            return_errno_with_message!(Errno::EAGAIN, "no pending connection is available");
        };
        self.num_conns
            .store(incoming_conns.len(), Ordering::Relaxed);

        drop(incoming_conns);
        self.pollee.invalidate();

        Ok(connection)
    }

    pub(self) fn set_backlog(&self, backlog: usize) {
        self.backlog
            .store(backlog.min(MAX_BACKLOG), Ordering::Relaxed);
    }

    pub(self) fn check_io_events(&self) -> IoEvents {
        let incoming_conns = self.incoming_conns.lock();

        if incoming_conns.is_empty() {
            IoEvents::empty()
        } else {
            IoEvents::IN
        }
    }

    pub(super) fn take_incoming_on_removal(&self) -> VecDeque<Arc<ConnectionInner>> {
        core::mem::take(&mut *self.incoming_conns.lock())
    }

    pub(super) fn bound_port(&self) -> &BoundPort {
        &self.bound_port
    }
}
