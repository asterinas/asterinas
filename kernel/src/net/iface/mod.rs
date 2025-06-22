// SPDX-License-Identifier: MPL-2.0

mod ext;
mod init;
mod poll;
mod sched;

pub use init::{init, iter_all_ifaces, loopback_iface, virtio_iface};
pub use poll::lazy_init;

pub type Iface = dyn aster_bigtcp::iface::Iface<ext::BigtcpExt>;
pub type BoundPort = aster_bigtcp::iface::BoundPort<ext::BigtcpExt>;

pub type RawTcpSocketExt = aster_bigtcp::socket::RawTcpSocketExt<ext::BigtcpExt>;

pub type TcpConnection = aster_bigtcp::socket::TcpConnection<ext::BigtcpExt>;
pub type TcpListener = aster_bigtcp::socket::TcpListener<ext::BigtcpExt>;
pub type UdpSocket = aster_bigtcp::socket::UdpSocket<ext::BigtcpExt>;
