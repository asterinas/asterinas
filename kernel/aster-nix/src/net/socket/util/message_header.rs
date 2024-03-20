// SPDX-License-Identifier: MPL-2.0

use super::socket_addr::SocketAddr;
use crate::{prelude::*, util::IoVec};

/// Message header used for sendmsg/recvmsg.
#[derive(Debug)]
pub struct MessageHeader {
    pub(in crate::net) addr: Option<SocketAddr>,
    pub(in crate::net) control_message: Option<ControlMessage>,
}

impl MessageHeader {
    /// Creates a new `MessageHeader`.
    pub const fn new(addr: Option<SocketAddr>, control_message: Option<ControlMessage>) -> Self {
        Self {
            addr,
            control_message,
        }
    }

    /// Returns the socket address.
    pub fn addr(&self) -> Option<&SocketAddr> {
        self.addr.as_ref()
    }
}

/// Control message carried by MessageHeader.
///
/// TODO: Implement the struct. The struct is empty now.
#[derive(Debug)]
pub struct ControlMessage;

/// Copies a message from user space.
///
/// Since udp allows sending and receiving packet of length 0,
/// The returned buffer may have length of zero.
pub fn copy_message_from_user(io_vecs: &[IoVec]) -> Box<[u8]> {
    let mut buffer = create_message_buffer(io_vecs);

    let mut total_bytes = 0;
    for io_vec in io_vecs {
        if io_vec.is_empty() {
            continue;
        }
        let dst = &mut buffer[total_bytes..total_bytes + io_vec.len()];
        // FIXME: short read should be allowed here
        match io_vec.read_exact_from_user(dst) {
            Ok(()) => total_bytes += io_vec.len(),
            Err(e) => {
                warn!("fails to copy message from user");
                break;
            }
        }
    }

    buffer.truncate(total_bytes);
    buffer.into_boxed_slice()
}

/// Creates a buffer whose length
/// is equal to the total length of `io_vecs`.
pub fn create_message_buffer(io_vecs: &[IoVec]) -> Vec<u8> {
    let buffer_len: usize = io_vecs.iter().map(|iovec| iovec.len()).sum();
    vec![0; buffer_len]
}

/// Copies a message to user space.
///
/// This method returns the actual copied length.
pub fn copy_message_to_user(io_vecs: &[IoVec], message: &[u8]) -> usize {
    let mut total_bytes = 0;

    for io_vec in io_vecs {
        if io_vec.is_empty() {
            continue;
        }

        let len = io_vec.len().min(message.len() - total_bytes);
        if len == 0 {
            break;
        }

        let src = &message[total_bytes..total_bytes + len];
        match io_vec.write_to_user(src) {
            Ok(len) => total_bytes += len,
            Err(e) => {
                warn!("fails to copy message to user");
                break;
            }
        }
    }

    total_bytes
}
