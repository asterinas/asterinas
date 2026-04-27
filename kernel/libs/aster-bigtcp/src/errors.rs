// SPDX-License-Identifier: MPL-2.0

/// An error describing the reason why `bind` failed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BindError {
    /// All ephemeral ports is exhausted.
    Exhausted,
    /// The specified address is in use.
    InUse,
}

pub mod tcp {
    /// An error returned by a TCP stream I/O operation before any byte is transferred.
    ///
    /// If some bytes are transferred before a socket or copy error is observed, the
    /// operation succeeds with the transferred byte count instead.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum IoError<SocketError, CopyError> {
        /// The operation made no progress.
        ///
        /// This usually means the send buffer is full or the receive buffer is empty.
        NoProgress,
        /// The underlying TCP socket failed the operation.
        Socket(SocketError),
        /// The caller-provided copy function failed.
        Copy(CopyError),
    }

    /// An error returned by [`TcpListener::new_listen`].
    ///
    /// [`TcpListener::new_listen`]: crate::socket::TcpListener::new_listen
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
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
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
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
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
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

    impl<CopyError> From<smoltcp::socket::tcp::SendError> for IoError<SendError, CopyError> {
        fn from(value: smoltcp::socket::tcp::SendError) -> Self {
            Self::Socket(value.into())
        }
    }

    /// An error returned by [`TcpConnection::recv`].
    ///
    /// [`TcpConnection::recv`]: crate::socket::TcpConnection::recv
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
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

    impl<CopyError> From<smoltcp::socket::tcp::RecvError> for IoError<RecvError, CopyError> {
        fn from(value: smoltcp::socket::tcp::RecvError) -> Self {
            Self::Socket(value.into())
        }
    }
}

pub mod udp {
    pub use smoltcp::socket::udp::RecvError;

    /// An error returned by [`UdpSocket::send`].
    ///
    /// [`UdpSocket::send`]: crate::socket::UdpSocket::send
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
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
