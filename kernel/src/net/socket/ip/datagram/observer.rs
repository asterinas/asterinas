// SPDX-License-Identifier: MPL-2.0

use aster_bigtcp::socket::{SocketEventObserver, SocketEvents};

use crate::{events::IoEvents, process::signal::Pollee};

pub struct DatagramObserver(Pollee);

impl DatagramObserver {
    pub(super) fn new(pollee: Pollee) -> Self {
        Self(pollee)
    }
}

impl SocketEventObserver for DatagramObserver {
    fn on_events(&self, events: SocketEvents) {
        let mut io_events = IoEvents::empty();

        if events.contains(SocketEvents::CAN_RECV) {
            io_events |= IoEvents::IN;
        }

        if events.contains(SocketEvents::CAN_SEND) {
            io_events |= IoEvents::OUT;
        }

        self.0.notify(io_events);
    }
}
