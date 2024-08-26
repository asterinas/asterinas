// SPDX-License-Identifier: MPL-2.0

// Modified from error.rs in virtio-drivers project
//
// MIT License
//
// Copyright (c) 2022-2023 Ant Group
// Copyright (c) 2019-2020 rCore Developers
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in all
// copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
// SOFTWARE.
//
use core::{fmt, result};

use crate::queue::QueueError;

/// The error type of VirtIO socket driver.
#[derive(Debug)]
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
    QueueError(QueueError),
}

impl From<QueueError> for SocketError {
    fn from(value: QueueError) -> Self {
        Self::QueueError(value)
    }
}

impl From<int_to_c_enum::TryFromIntError> for SocketError {
    fn from(_e: int_to_c_enum::TryFromIntError) -> Self {
        Self::InvalidNumber
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
            Self::QueueError(_) => write!(f,"Error encountered out of vsock itself!"),
        }
    }
}

pub type Result<T> = result::Result<T, SocketError>;
