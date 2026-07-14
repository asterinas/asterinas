// SPDX-License-Identifier: MPL-2.0

pub(super) mod datagram_common;
mod linger_option;
mod message_flags;
mod message_header;
pub(super) mod options;
mod port_privilege;
mod shutdown_cmd;
mod socket_addr;
mod socket_timeout;

pub use linger_option::LingerOption;
pub use message_flags::{RecvFlags, SendFlags};
pub(super) use message_header::CControlHeader;
pub use message_header::{ControlMessage, MessageHeader};
pub(super) use port_privilege::check_port_privilege;
pub use shutdown_cmd::SockShutdownCmd;
pub use socket_addr::SocketAddr;
pub use socket_timeout::SocketTimeout;
