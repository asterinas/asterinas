// SPDX-License-Identifier: MPL-2.0

/// An error describing the reason why `bind` failed.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum BindError {
    /// All ephemeral ports is exhausted.
    Exhausted,
    /// The specified address is in use.
    InUse,
}

pub mod tcp {
    pub use smoltcp::socket::tcp::{ConnectError, ListenError, RecvError, SendError};
}

pub mod udp {
    pub use smoltcp::socket::udp::RecvError;

    /// An error returned by [`BoundTcpSocket::recv`].
    ///
    /// [`BoundTcpSocket::recv`]: crate::socket::BoundTcpSocket::recv
    #[derive(Debug, PartialEq, Eq, Clone, Copy)]
    pub enum SendError {
        TooLarge,
        Unaddressable,
        BufferFull,
    }
}
