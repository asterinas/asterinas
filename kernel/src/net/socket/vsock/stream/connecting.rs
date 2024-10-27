// SPDX-License-Identifier: MPL-2.0

use aster_virtio::device::socket::connect::{ConnectionInfo, VsockEvent};

use super::connected::ConnectionID;
use crate::{
    events::IoEvents,
    net::socket::vsock::{addr::VsockSocketAddr, VSOCK_GLOBAL},
    prelude::*,
    process::signal::{Pollee, Poller},
};

pub struct Connecting {
    id: ConnectionID,
    info: SpinLock<ConnectionInfo>,
    pollee: Pollee,
}

impl Connecting {
    pub fn new(peer_addr: VsockSocketAddr, local_addr: VsockSocketAddr) -> Self {
        Self {
            info: SpinLock::new(ConnectionInfo::new(peer_addr.into(), local_addr.port)),
            id: ConnectionID::new(local_addr, peer_addr),
            pollee: Pollee::new(IoEvents::empty()),
        }
    }

    pub fn peer_addr(&self) -> VsockSocketAddr {
        self.id.peer_addr
    }

    pub fn local_addr(&self) -> VsockSocketAddr {
        self.id.local_addr
    }

    pub fn id(&self) -> ConnectionID {
        self.id
    }

    pub fn info(&self) -> ConnectionInfo {
        self.info.disable_irq().lock_with(|info| info.clone())
    }

    pub fn update_info(&self, event: &VsockEvent) {
        self.info
            .disable_irq()
            .lock_with(|info| info.update_for_event(event));
    }

    pub fn poll(&self, mask: IoEvents, poller: Option<&mut Poller>) -> IoEvents {
        self.pollee.poll(mask, poller)
    }

    pub fn add_events(&self, events: IoEvents) {
        self.pollee.add_events(events)
    }
}

impl Drop for Connecting {
    fn drop(&mut self) {
        let vsockspace = VSOCK_GLOBAL.get().unwrap();
        vsockspace.recycle_port(&self.local_addr().port);
        vsockspace.remove_connecting_socket(&self.local_addr());
    }
}
