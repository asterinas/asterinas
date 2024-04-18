// SPDX-License-Identifier: MPL-2.0

use super::connected::Connected;
use crate::{
    events::IoEvents,
    net::socket::vsock::{addr::VsockSocketAddr, VSOCK_GLOBAL},
    prelude::*,
    process::signal::{Pollee, Poller},
};
pub struct Listen {
    addr: VsockSocketAddr,
    pollee: Pollee,
    backlog: usize,
    incoming_connection: SpinLock<VecDeque<Arc<Connected>>>,
}

impl Listen {
    pub fn new(addr: VsockSocketAddr, backlog: usize) -> Self {
        Self {
            addr,
            pollee: Pollee::new(IoEvents::empty()),
            backlog,
            incoming_connection: SpinLock::new(VecDeque::with_capacity(backlog)),
        }
    }

    pub fn addr(&self) -> VsockSocketAddr {
        self.addr
    }
    pub fn push_incoming(&self, connect: Arc<Connected>) -> Result<()> {
        let mut incoming_connections = self.incoming_connection.lock_irq_disabled();
        if incoming_connections.len() >= self.backlog {
            return_errno_with_message!(Errno::ENOMEM, "Queue in listenging socket is full")
        }
        incoming_connections.push_back(connect);
        self.add_events(IoEvents::IN);
        Ok(())
    }
    pub fn accept(&self) -> Result<Arc<Connected>> {
        // block waiting connection if no existing connection.
        let poller = Poller::new();
        if !self
            .poll(IoEvents::IN, Some(&poller))
            .contains(IoEvents::IN)
        {
            poller.wait()?;
        }

        let connection = self
            .incoming_connection
            .lock_irq_disabled()
            .pop_front()
            .ok_or_else(|| {
                Error::with_message(Errno::EAGAIN, "no pending connection is available")
            })?;

        Ok(connection)
    }

    pub fn poll(&self, mask: IoEvents, poller: Option<&Poller>) -> IoEvents {
        self.pollee.poll(mask, poller)
    }
    pub fn add_events(&self, events: IoEvents) {
        self.pollee.add_events(events)
    }
}

impl Drop for Listen {
    fn drop(&mut self) {
        VSOCK_GLOBAL
            .get()
            .unwrap()
            .used_ports
            .lock_irq_disabled()
            .remove(&self.addr.port);
    }
}
