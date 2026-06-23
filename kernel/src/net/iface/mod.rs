// SPDX-License-Identifier: MPL-2.0

mod broadcast;
mod ext;
mod init;
mod poll;
mod sched;

pub use broadcast::is_broadcast_endpoint;
pub use init::{init, iter_all_ifaces, loopback_iface, virtio_iface};
pub(super) use poll::init_in_first_kthread;

pub type Iface = dyn aster_bigtcp::iface::Iface<ext::BigtcpExt>;
pub type BoundTcpPort = aster_bigtcp::iface::BoundTcpPort<ext::BigtcpExt>;
pub type BoundUdpPort = aster_bigtcp::iface::BoundUdpPort<ext::BigtcpExt>;
pub type BoundRawPort = aster_bigtcp::iface::BoundRawPort<ext::BigtcpExt>;

pub type RawTcpSocketExt = aster_bigtcp::socket::RawTcpSocketExt<ext::BigtcpExt>;

pub type TcpConnection = aster_bigtcp::socket::TcpConnection<ext::BigtcpExt>;
pub type TcpListener = aster_bigtcp::socket::TcpListener<ext::BigtcpExt>;
pub type UdpSocket = aster_bigtcp::socket::UdpSocket<ext::BigtcpExt>;
pub type RawSocket = aster_bigtcp::socket::RawSocket<ext::BigtcpExt>;
pub type IcmpSocket = aster_bigtcp::socket::IcmpSocket<ext::BigtcpExt>;
pub type BoundIcmpPort = aster_bigtcp::iface::BoundIcmpPort<ext::BigtcpExt>;
