// SPDX-License-Identifier: MPL-2.0

use smoltcp::time::Duration;

use super::NeedIfacePoll;

/// A trait defines setting socket options on a raw socket.
///
/// TODO: When `UnboundSocket` is removed, all methods in this trait can accept
/// `&self` instead of `&mut self` as parameter.
pub trait RawTcpSetOption {
    /// Sets the keep alive interval.
    ///
    /// Polling the iface _may_ be required after this method succeeds.
    fn set_keep_alive(&mut self, interval: Option<Duration>) -> NeedIfacePoll;

    /// Enables or disables Nagle’s Algorithm.
    ///
    /// Polling the iface is not required after this method succeeds.
    fn set_nagle_enabled(&mut self, enabled: bool);
}
