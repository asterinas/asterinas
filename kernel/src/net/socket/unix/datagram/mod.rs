// SPDX-License-Identifier: MPL-2.0

mod message;
mod socket;

pub(in crate::net) use message::UNIX_DATAGRAM_DEFAULT_BUF_SIZE;
pub use socket::UnixDatagramSocket;
