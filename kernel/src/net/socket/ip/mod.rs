// SPDX-License-Identifier: MPL-2.0

mod addr;
mod common;
mod datagram;
pub mod options;
mod stream;

pub(in crate::net) use datagram::observer::DatagramObserver;
pub use datagram::DatagramSocket;
pub(in crate::net) use stream::observer::StreamObserver;
pub use stream::{options as stream_options, StreamSocket};
