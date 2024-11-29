// SPDX-License-Identifier: MPL-2.0

mod bound;
mod event;
mod state;
mod unbound;

pub use bound::{BoundRawSocket, BoundTcpSocket, BoundUdpSocket, ConnectState, NeedIfacePoll};
pub(crate) use bound::{
    BoundRawSocketInner, BoundTcpSocketInner, BoundUdpSocketInner, TcpProcessResult,
};
pub use event::{SocketEventObserver, SocketEvents};
pub use state::TcpStateCheck;
pub use unbound::{
    UnboundRawSocket, UnboundTcpSocket, UnboundUdpSocket, RAW_RECV_PAYLOAD_LEN,
    RAW_SEND_PAYLOAD_LEN, TCP_RECV_BUF_LEN, TCP_SEND_BUF_LEN, UDP_RECV_PAYLOAD_LEN,
    UDP_SEND_PAYLOAD_LEN,
};

pub type NativeTcpSocket = smoltcp::socket::tcp::Socket<'static>;
pub type NativeUdpSocket = smoltcp::socket::udp::Socket<'static>;
pub type NativeRawSocket = smoltcp::socket::raw::Socket<'static>;
