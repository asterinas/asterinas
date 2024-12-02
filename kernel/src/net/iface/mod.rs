// SPDX-License-Identifier: MPL-2.0

mod ext;
mod init;
mod poll;

pub use init::{init, IFACES};
pub use poll::lazy_init;

pub type Iface = dyn aster_bigtcp::iface::Iface<ext::IfaceExt>;
pub type BoundPort = aster_bigtcp::iface::BoundPort<ext::IfaceExt>;

pub type TcpConnection = aster_bigtcp::socket::TcpConnection<ext::IfaceExt>;
pub type TcpListener = aster_bigtcp::socket::TcpListener<ext::IfaceExt>;
pub type UdpSocket = aster_bigtcp::socket::UdpSocket<ext::IfaceExt>;
