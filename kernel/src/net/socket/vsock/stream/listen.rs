// SPDX-License-Identifier: MPL-2.0

use super::connected::Connected;
use crate::{
    events::IoEvents,
    net::socket::vsock::addr::VsockSocketAddr,
    prelude::*,
    process::signal::{PollHandle, Pollee},
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
            // FIXME: We should reuse `Pollee` from `Init`.
            pollee: Pollee::new(),
            backlog,
            incoming_connection: SpinLock::new(VecDeque::with_capacity(backlog)),
        }
    }

    pub fn addr(&self) -> VsockSocketAddr {
        self.addr
    }

    pub fn push_incoming(&self, connect: Arc<Connected>) -> Result<()> {
        let mut incoming_connections = self.incoming_connection.disable_irq().lock();
        if incoming_connections.len() >= self.backlog {
            return_errno_with_message!(Errno::ECONNREFUSED, "queue in listenging socket is full")
        }

        // FIXME: check if the port is already used
        incoming_connections.push_back(connect);
        self.pollee.notify(IoEvents::IN);

        Ok(())
    }

    pub fn try_accept(&self) -> Result<Arc<Connected>> {
        let connection = self
            .incoming_connection
            .disable_irq()
            .lock()
            .pop_front()
            .ok_or_else(|| {
                Error::with_message(Errno::EAGAIN, "no pending connection is available")
            })?;
        self.pollee.invalidate();

        Ok(connection)
    }

    pub fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        self.pollee
            .poll_with(mask, poller, || self.check_io_events())
    }

    fn check_io_events(&self) -> IoEvents {
        let incoming_connection = self.incoming_connection.disable_irq().lock();

        if !incoming_connection.is_empty() {
            IoEvents::IN
        } else {
            IoEvents::empty()
        }
    }
}
