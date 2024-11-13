// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicBool, Ordering};

use aster_virtio::device::socket::connect::{ConnectionInfo, VsockEvent};

use super::connected::ConnectionID;
use crate::{
    events::IoEvents,
    net::socket::vsock::{addr::VsockSocketAddr, VSOCK_GLOBAL},
    prelude::*,
    process::signal::{PollHandle, Pollee},
};

pub struct Connecting {
    id: ConnectionID,
    info: SpinLock<ConnectionInfo>,
    is_connected: AtomicBool,
    pollee: Pollee,
}

impl Connecting {
    pub fn new(peer_addr: VsockSocketAddr, local_addr: VsockSocketAddr) -> Self {
        Self {
            info: SpinLock::new(ConnectionInfo::new(peer_addr.into(), local_addr.port)),
            id: ConnectionID::new(local_addr, peer_addr),
            is_connected: AtomicBool::new(false),
            pollee: Pollee::new(),
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
        self.info.disable_irq().lock().clone()
    }

    pub fn update_info(&self, event: &VsockEvent) {
        self.info.disable_irq().lock().update_for_event(event)
    }

    pub fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        self.pollee
            .poll_with(mask, poller, || self.check_io_events())
    }

    fn check_io_events(&self) -> IoEvents {
        if self.is_connected.load(Ordering::Relaxed) {
            IoEvents::IN
        } else {
            IoEvents::empty()
        }
    }

    pub fn set_connected(&self) {
        self.is_connected.store(true, Ordering::Relaxed);
        self.pollee.notify(IoEvents::IN);
    }
}

impl Drop for Connecting {
    fn drop(&mut self) {
        let vsockspace = VSOCK_GLOBAL.get().unwrap();
        vsockspace.recycle_port(&self.local_addr().port);
        vsockspace.remove_connecting_socket(&self.local_addr());
    }
}
