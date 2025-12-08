// SPDX-License-Identifier: MPL-2.0

mod addr;
mod common;
mod datagram;
pub mod options;
mod stream;

pub use datagram::DatagramSocket;
pub(in crate::net) use datagram::observer::DatagramObserver;
pub(in crate::net) use stream::observer::StreamObserver;
pub use stream::{StreamSocket, options as stream_options};
