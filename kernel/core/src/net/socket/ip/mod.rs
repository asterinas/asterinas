// SPDX-License-Identifier: MPL-2.0

mod addr;
mod common;
mod datagram;
mod ioctl;
pub(crate) mod options;
mod stream;

pub(crate) use addr::IpAddressFamily;
pub(crate) use datagram::DatagramSocket;
pub(in crate::net) use datagram::observer::DatagramObserver;
pub(in crate::net) use stream::observer::StreamObserver;
pub(crate) use stream::{StreamSocket, options as stream_options};
