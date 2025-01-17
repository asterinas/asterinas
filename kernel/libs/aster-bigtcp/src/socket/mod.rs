// SPDX-License-Identifier: MPL-2.0

mod bound;
mod event;
mod option;
mod state;
mod unbound;

pub use bound::{ConnectState, NeedIfacePoll, RawSocket, TcpConnection, TcpListener, UdpSocket};
pub(crate) use bound::{
    RawSocketBg, TcpConnectionBg, TcpListenerBg, TcpProcessResult, UdpSocketBg,
};
pub use event::{SocketEventObserver, SocketEvents};
pub use option::{SmolTcpOption, SmolTcpSetOption};
pub use state::TcpStateCheck;
pub use unbound::{
    RAW_RECV_PAYLOAD_LEN, RAW_SEND_PAYLOAD_LEN, TCP_RECV_BUF_LEN, TCP_SEND_BUF_LEN,
    UDP_RECV_PAYLOAD_LEN, UDP_SEND_PAYLOAD_LEN,
};

pub type SmolTcpSocket = smoltcp::socket::tcp::Socket<'static>;
pub type SmolUdpSocket = smoltcp::socket::udp::Socket<'static>;
pub type SmolRawSocket = smoltcp::socket::raw::Socket<'static>;
