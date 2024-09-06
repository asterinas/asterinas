// SPDX-License-Identifier: MPL-2.0

mod addr;
mod common;
mod datagram;
pub mod stream;

use addr::UNSPECIFIED_LOCAL_ENDPOINT;
pub use datagram::DatagramSocket;
pub use stream::StreamSocket;
