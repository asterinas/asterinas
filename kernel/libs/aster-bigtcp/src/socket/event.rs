// SPDX-License-Identifier: MPL-2.0

/// A observer that will be invoked whenever events occur on the socket.
pub trait SocketEventObserver: Send + Sync {
    /// Notifies that events occurred on the socket.
    fn on_events(&self);
}

impl SocketEventObserver for () {
    fn on_events(&self) {}
}
