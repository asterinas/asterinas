// SPDX-License-Identifier: MPL-2.0

use aster_bigtcp::socket::{SocketEventObserver, SocketEvents};

use crate::{events::IoEvents, process::signal::Pollee};

#[derive(Clone)]
pub struct StreamObserver(Pollee);

impl StreamObserver {
    pub(super) fn new(pollee: Pollee) -> Self {
        Self(pollee)
    }
}

impl SocketEventObserver for StreamObserver {
    fn on_events(&self, events: SocketEvents) {
        let mut io_events = IoEvents::empty();

        if events.contains(SocketEvents::CAN_RECV) {
            io_events |= IoEvents::IN;
        }

        if events.contains(SocketEvents::CAN_SEND) {
            io_events |= IoEvents::OUT;
        }

        if events.contains(SocketEvents::CLOSED_RECV) {
            // `CLOSED_RECV` definitely causes IN and RDHUP.
            io_events |= IoEvents::IN | IoEvents::RDHUP;
            // `CLOSED_RECV` may cause HUP/ERR (combined with a previous `CLOSED_SEND`).
            io_events |= IoEvents::HUP | IoEvents::ERR;
        }

        if events.contains(SocketEvents::CLOSED_SEND) {
            // `CLOSED_SEND` definitely causes OUT.
            io_events |= IoEvents::OUT;
            // `CLOSED_SEND` may cause HUP/ERR (combined with a previous `CLOSED_RECV`).
            io_events |= IoEvents::HUP | IoEvents::ERR;
        }

        self.0.notify(io_events);
    }
}
