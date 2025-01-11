// SPDX-License-Identifier: MPL-2.0

mod bound;
mod event;
mod option;
mod state;
mod unbound;

pub use bound::{ConnectState, NeedIfacePoll, TcpConnection, TcpListener, UdpSocket};
pub(crate) use bound::{TcpConnectionBg, TcpListenerBg, TcpProcessResult, UdpSocketBg};
pub use event::{SocketEventObserver, SocketEvents};
pub use option::{RawTcpOption, RawTcpSetOption};
pub use state::TcpStateCheck;
pub use unbound::{TCP_RECV_BUF_LEN, TCP_SEND_BUF_LEN, UDP_RECV_PAYLOAD_LEN, UDP_SEND_PAYLOAD_LEN};

pub type RawTcpSocket = smoltcp::socket::tcp::Socket<'static>;
pub type RawUdpSocket = smoltcp::socket::udp::Socket<'static>;
