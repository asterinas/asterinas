// SPDX-License-Identifier: MPL-2.0

/// A observer that will be invoked whenever events occur on the socket.
pub trait SocketEventObserver: Send + Sync {
    /// Notifies that events occurred on the socket.
    fn on_events(&self, events: SocketEvents);
}

impl SocketEventObserver for () {
    fn on_events(&self, _events: SocketEvents) {}
}

bitflags::bitflags! {
    /// Socket events caused by the _network_.
    pub struct SocketEvents: u8 {
        const CAN_RECV = 1;
        const CAN_SEND = 2;
        /// Receiving new data isn't possible anymore.
        const CLOSED_RECV = 4;
        /// Sending data isn't possible anymore.
        const CLOSED_SEND = 8;
    }
}
