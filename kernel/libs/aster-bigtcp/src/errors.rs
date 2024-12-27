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
    pub use smoltcp::socket::tcp::{RecvError, SendError};

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
