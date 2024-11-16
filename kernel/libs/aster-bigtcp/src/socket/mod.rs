// SPDX-License-Identifier: MPL-2.0

mod bound;
mod event;
mod state;
mod unbound;

pub use bound::{BoundTcpSocket, BoundUdpSocket, ConnectState, NeedIfacePoll};
pub(crate) use bound::{BoundTcpSocketInner, BoundUdpSocketInner, TcpProcessResult};
pub use event::{SocketEventObserver, SocketEvents};
pub use state::TcpStateCheck;
pub use unbound::{
    UnboundTcpSocket, UnboundUdpSocket, TCP_RECV_BUF_LEN, TCP_SEND_BUF_LEN, UDP_RECV_PAYLOAD_LEN,
    UDP_SEND_PAYLOAD_LEN,
};

pub type RawTcpSocket = smoltcp::socket::tcp::Socket<'static>;
pub type RawUdpSocket = smoltcp::socket::udp::Socket<'static>;
