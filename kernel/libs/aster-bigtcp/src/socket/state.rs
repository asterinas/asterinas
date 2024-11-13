// SPDX-License-Identifier: MPL-2.0

use smoltcp::socket::tcp::State as TcpState;

use super::RawTcpSocket;

pub trait TcpStateCheck {
    /// Checks if the peer socket has closed its sending side.
    ///
    /// If the sending side of this socket is also closed, this method will return `false`.
    /// In such cases, you should verify using [`is_closed`].
    fn is_peer_closed(&self) -> bool;

    /// Checks if the socket is fully closed.
    ///
    /// This function returns `true` if both this socket and the peer have closed their sending sides.
    ///
    /// This TCP state corresponds to the `Normal Close Sequence` and `Simultaneous Close Sequence`
    /// as outlined in RFC793 (https://datatracker.ietf.org/doc/html/rfc793#page-39).
    fn is_closed(&self) -> bool;
}

impl TcpStateCheck for RawTcpSocket {
    fn is_peer_closed(&self) -> bool {
        self.state() == TcpState::CloseWait
    }

    fn is_closed(&self) -> bool {
        !self.is_open() || self.state() == TcpState::Closing || self.state() == TcpState::LastAck
    }
}
