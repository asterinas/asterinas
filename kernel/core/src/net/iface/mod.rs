// SPDX-License-Identifier: MPL-2.0

mod ext;
mod init;
mod poll;
mod sched;

pub(super) use init::{init, iter_all_ifaces};
pub(super) use poll::init_in_first_kthread;

pub(super) type Iface = dyn aster_bigtcp::iface::Iface<ext::BigtcpExt>;
pub(super) type BoundTcpPort = aster_bigtcp::iface::BoundTcpPort<ext::BigtcpExt>;
pub(super) type BoundUdpPort = aster_bigtcp::iface::BoundUdpPort<ext::BigtcpExt>;

pub(super) type RawTcpSocketExt = aster_bigtcp::socket::RawTcpSocketExt<ext::BigtcpExt>;

pub(super) type TcpConnection = aster_bigtcp::socket::TcpConnection<ext::BigtcpExt>;
pub(super) type TcpListener = aster_bigtcp::socket::TcpListener<ext::BigtcpExt>;
pub(super) type UdpSocket = aster_bigtcp::socket::UdpSocket<ext::BigtcpExt>;

/// The default transmit queue length.
///
/// On Linux, this value limits the number of SKBs
/// that can be queued in a network device's egress qdisc.
/// This value does not take effect on Asterinas now.
///
/// Reference: <https://elixir.bootlin.com/linux/v7.1/source/include/net/pkt_sched.h#L13>.
pub(super) const DEFAULT_TX_QUEUE_LEN: u32 = 1000;
