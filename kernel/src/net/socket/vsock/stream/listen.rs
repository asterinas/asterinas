// SPDX-License-Identifier: MPL-2.0

use super::connected::Connected;
use crate::{
    events::IoEvents,
    net::socket::vsock::addr::VsockSocketAddr,
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
        self.incoming_connection
            .disable_irq()
            .lock_with(|incoming_connections| {
                if incoming_connections.len() >= self.backlog {
                    return_errno_with_message!(
                        Errno::ECONNREFUSED,
                        "queue in listenging socket is full"
                    )
                }
                // FIXME: check if the port is already used
                incoming_connections.push_back(connect);
                Ok(())
            })
    }

    pub fn try_accept(&self) -> Result<Arc<Connected>> {
        let connection = self
            .incoming_connection
            .disable_irq()
            .lock_with(|connection| {
                connection.pop_front().ok_or_else(|| {
                    Error::with_message(Errno::EAGAIN, "no pending connection is available")
                })
            })?;

        Ok(connection)
    }

    pub fn poll(&self, mask: IoEvents, poller: Option<&mut Poller>) -> IoEvents {
        self.pollee.poll(mask, poller)
    }

    pub fn update_io_events(&self) {
        self.incoming_connection
            .disable_irq()
            .lock_with(|incoming_connection| {
                if !incoming_connection.is_empty() {
                    self.pollee.add_events(IoEvents::IN);
                } else {
                    self.pollee.del_events(IoEvents::IN);
                }
            });
    }
}
