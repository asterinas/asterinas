// SPDX-License-Identifier: MPL-2.0

mod common;
mod tcp_conn;
mod tcp_listen;
mod udp;

pub use common::NeedIfacePoll;
pub use tcp_conn::{ConnectState, RawTcpSocketExt, TcpConnection};
pub(crate) use tcp_conn::{TcpConnectionBg, TcpProcessResult};
pub use tcp_listen::TcpListener;
pub(crate) use tcp_listen::TcpListenerBg;
pub use udp::UdpSocket;
pub(crate) use udp::UdpSocketBg;

/// Describes how receive operations consume queued data.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReceiveBehavior {
    /// Removes received data from the receive buffer.
    Recv,
    /// Leaves received data in the receive buffer.
    Peek,
}

impl ReceiveBehavior {
    /// Returns whether received data should be removed from the receive buffer.
    pub fn will_consume_data(self) -> bool {
        self == Self::Recv
    }
}
