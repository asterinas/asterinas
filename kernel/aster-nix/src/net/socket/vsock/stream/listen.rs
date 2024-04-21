// SPDX-License-Identifier: MPL-2.0

use super::connected::Connected;
use crate::{
    events::{IoEvents, Observer},
    net::socket::vsock::{addr::VsockSocketAddr, VSOCK_GLOBAL},
    prelude::*,
    process::signal::{Pollee, Poller},
};
pub struct Listen {
    addr: VsockSocketAddr,
    backlog: usize,
    incoming_connection: SpinLock<VecDeque<Arc<Connected>>>,
    pollee: Pollee,
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
        Ok(())
    }

    pub fn try_accept(&self) -> Result<Arc<Connected>> {
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

    pub fn update_io_events(&self) {
        let can_accept = !self.incoming_connection.lock_irq_disabled().is_empty();
        if can_accept {
            self.pollee.add_events(IoEvents::IN);
        } else {
            self.pollee.del_events(IoEvents::IN);
        }
    }
    pub fn register_observer(
        &self,
        pollee: &Pollee,
        observer: Weak<dyn Observer<IoEvents>>,
        mask: IoEvents,
    ) -> Result<()> {
        pollee.register_observer(observer, mask);
        Ok(())
    }

    pub fn unregister_observer(
        &self,
        pollee: &Pollee,
        observer: &Weak<dyn Observer<IoEvents>>,
    ) -> Result<Weak<dyn Observer<IoEvents>>> {
        pollee
            .unregister_observer(observer)
            .ok_or_else(|| Error::with_message(Errno::EINVAL, "fails to unregister observer"))
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
