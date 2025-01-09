// SPDX-License-Identifier: MPL-2.0

use smoltcp::time::Duration;

use super::{NeedIfacePoll, SmolTcpSocket};

/// A trait defines setting socket options on a smoltcp socket.
pub trait SmolTcpSetOption {
    /// Sets the keep alive interval.
    ///
    /// Polling the iface _may_ be required after this method succeeds.
    fn set_keep_alive(&self, interval: Option<Duration>) -> NeedIfacePoll;

    /// Enables or disables Nagle’s Algorithm.
    ///
    /// Polling the iface is _not_ required after this method succeeds.
    fn set_nagle_enabled(&self, enabled: bool);
}

/// Socket options on a smoltcp socket.
pub struct SmolTcpOption {
    /// The keep alive interval.
    pub keep_alive: Option<Duration>,
    /// Whether Nagle's algorithm is enabled.
    pub is_nagle_enabled: bool,
}

impl SmolTcpOption {
    pub(super) fn apply(&self, socket: &mut SmolTcpSocket) {
        socket.set_keep_alive(self.keep_alive);
        socket.set_nagle_enabled(self.is_nagle_enabled);
    }

    pub(super) fn inherit(from: &SmolTcpSocket, to: &mut SmolTcpSocket) {
        to.set_keep_alive(from.keep_alive());
        to.set_nagle_enabled(from.nagle_enabled());
    }
}
