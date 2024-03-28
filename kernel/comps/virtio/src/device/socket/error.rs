// SPDX-License-Identifier: MPL-2.0

//! This file comes from virtio-drivers project
//! This module contains the error from the VirtIO socket driver.

use core::{fmt, result};

use crate::queue::QueueError;

/// The error type of VirtIO socket driver.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SocketError {
    /// There is an existing connection.
    ConnectionExists,
    /// Failed to establish the connection.
    ConnectionFailed,
    /// The device is not connected to any peer.
    NotConnected,
    /// Peer socket is shutdown.
    PeerSocketShutdown,
    /// No response received.
    NoResponseReceived,
    /// The given buffer is shorter than expected.
    BufferTooShort,
    /// The given buffer for output is shorter than expected.
    OutputBufferTooShort(usize),
    /// The given buffer has exceeded the maximum buffer size.
    BufferTooLong(usize, usize),
    /// Unknown operation.
    UnknownOperation(u16),
    /// Invalid operation,
    InvalidOperation,
    /// Invalid number.
    InvalidNumber,
    /// Unexpected data in packet.
    UnexpectedDataInPacket,
    /// Peer has insufficient buffer space, try again later.
    InsufficientBufferSpaceInPeer,
    /// Recycled a wrong buffer.
    RecycledWrongBuffer,
    /// Queue Error
    QueueError(SocketQueueError),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SocketQueueError {
    InvalidArgs,
    BufferTooSmall,
    NotReady,
    AlreadyUsed,
    WrongToken,
}

impl From<QueueError> for SocketQueueError {
    fn from(value: QueueError) -> Self {
        match value {
            QueueError::InvalidArgs => Self::InvalidArgs,
            QueueError::BufferTooSmall => Self::BufferTooSmall,
            QueueError::NotReady => Self::NotReady,
            QueueError::AlreadyUsed => Self::AlreadyUsed,
            QueueError::WrongToken => Self::WrongToken,
        }
    }
}

impl From<QueueError> for SocketError {
    fn from(value: QueueError) -> Self {
        Self::QueueError(SocketQueueError::from(value))
    }
}

impl fmt::Display for SocketError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::ConnectionExists => write!(
                f,
                "There is an existing connection. Please close the current connection before attempting to connect again."),
            Self::ConnectionFailed => write!(
                f, "Failed to establish the connection. The packet sent may have an unknown type value"
            ),
            Self::NotConnected => write!(f, "The device is not connected to any peer. Please connect it to a peer first."),
            Self::PeerSocketShutdown => write!(f, "The peer socket is shutdown."),
            Self::NoResponseReceived => write!(f, "No response received"),
            Self::BufferTooShort => write!(f, "The given buffer is shorter than expected"),
            Self::BufferTooLong(actual, max) => {
                write!(f, "The given buffer length '{actual}' has exceeded the maximum allowed buffer length '{max}'")
            }
            Self::OutputBufferTooShort(expected) => {
                write!(f, "The given output buffer is too short. '{expected}' bytes is needed for the output buffer.")
            }
            Self::UnknownOperation(op) => {
                write!(f, "The operation code '{op}' is unknown")
            }
            Self::InvalidOperation => write!(f, "Invalid operation"),
            Self::InvalidNumber => write!(f, "Invalid number"),
            Self::UnexpectedDataInPacket => write!(f, "No data is expected in the packet"),
            Self::InsufficientBufferSpaceInPeer => write!(f, "Peer has insufficient buffer space, try again later"),
            Self::RecycledWrongBuffer => write!(f, "Recycled a wrong buffer"),
            Self::QueueError(_) => write!(f,"Error encounted out of vsock itself!"),
        }
    }
}

pub type Result<T> = result::Result<T, SocketError>;
