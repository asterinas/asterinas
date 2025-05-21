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
    /// An error returned by [`TcpListener::new_listen`].
    ///
    /// [`TcpListener::new_listen`]: crate::socket::TcpListener::new_listen
    #[derive(Debug, PartialEq, Eq, Clone, Copy)]
    pub enum ListenError {
        InvalidState,
        Unaddressable,
        /// The specified address is in use.
        AddressInUse,
    }

    impl From<smoltcp::socket::tcp::ListenError> for ListenError {
        fn from(value: smoltcp::socket::tcp::ListenError) -> Self {
            match value {
                smoltcp::socket::tcp::ListenError::InvalidState => Self::InvalidState,
                smoltcp::socket::tcp::ListenError::Unaddressable => Self::Unaddressable,
            }
        }
    }

    /// An error returned by [`TcpConnection::new_connect`].
    ///
    /// [`TcpConnection::new_connect`]: crate::socket::TcpConnection::new_connect
    #[derive(Debug, PartialEq, Eq, Clone, Copy)]
    pub enum ConnectError {
        InvalidState,
        Unaddressable,
        /// The specified address is in use.
        AddressInUse,
    }

    impl From<smoltcp::socket::tcp::ConnectError> for ConnectError {
        fn from(value: smoltcp::socket::tcp::ConnectError) -> Self {
            match value {
                smoltcp::socket::tcp::ConnectError::InvalidState => Self::InvalidState,
                smoltcp::socket::tcp::ConnectError::Unaddressable => Self::Unaddressable,
            }
        }
    }

    /// An error returned by [`TcpConnection::send`].
    ///
    /// [`TcpConnection::send`]: crate::socket::TcpConnection::send
    #[derive(Debug, PartialEq, Eq, Clone, Copy)]
    pub enum SendError {
        InvalidState,
        /// The connection is reset.
        ConnReset,
    }

    impl From<smoltcp::socket::tcp::SendError> for SendError {
        fn from(value: smoltcp::socket::tcp::SendError) -> Self {
            match value {
                smoltcp::socket::tcp::SendError::InvalidState => Self::InvalidState,
            }
        }
    }

    /// An error returned by [`TcpConnection::recv`].
    ///
    /// [`TcpConnection::recv`]: crate::socket::TcpConnection::recv
    #[derive(Debug, PartialEq, Eq, Clone, Copy)]
    pub enum RecvError {
        InvalidState,
        Finished,
        /// The connection is reset.
        ConnReset,
    }

    impl From<smoltcp::socket::tcp::RecvError> for RecvError {
        fn from(value: smoltcp::socket::tcp::RecvError) -> Self {
            match value {
                smoltcp::socket::tcp::RecvError::InvalidState => Self::InvalidState,
                smoltcp::socket::tcp::RecvError::Finished => Self::Finished,
            }
        }
    }
}

pub mod udp {
    pub use smoltcp::socket::udp::RecvError;

    /// An error returned by [`UdpSocket::send`].
    ///
    /// [`UdpSocket::send`]: crate::socket::UdpSocket::send
    #[derive(Debug, PartialEq, Eq, Clone, Copy)]
    pub enum SendError {
        Unaddressable,
        BufferFull,
        /// The packet is too large.
        TooLarge,
    }

    impl From<smoltcp::socket::udp::SendError> for SendError {
        fn from(value: smoltcp::socket::udp::SendError) -> Self {
            match value {
                smoltcp::socket::udp::SendError::Unaddressable => Self::Unaddressable,
                smoltcp::socket::udp::SendError::BufferFull => Self::BufferFull,
            }
        }
    }
}
