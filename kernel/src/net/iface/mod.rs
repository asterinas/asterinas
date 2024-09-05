// SPDX-License-Identifier: MPL-2.0

use ostd::sync::WaitQueue;

use self::common::IfaceCommon;
use crate::prelude::*;

mod any_socket;
mod common;
mod loopback;
mod time;
mod util;
mod virtio;

pub use any_socket::{
    AnyBoundSocket, AnyUnboundSocket, RawTcpSocket, RawUdpSocket, TCP_RECV_BUF_LEN,
    TCP_SEND_BUF_LEN, UDP_RECV_PAYLOAD_LEN, UDP_SEND_PAYLOAD_LEN,
};
pub use loopback::IfaceLoopback;
pub use smoltcp::wire::EthernetAddress;
pub use util::{spawn_background_poll_thread, BindPortConfig};
pub use virtio::IfaceVirtio;

use crate::net::socket::ip::Ipv4Address;

/// A network interface.
///
/// A network interface (abbreviated as iface) is a hardware or software component that connects a
/// computer to a network. Network interfaces can be physical components like Ethernet ports or
/// wireless adapters. They can also be virtual interfaces created by software, such as virtual
/// private network (VPN) connections.
pub trait Iface: internal::IfaceInternal + Send + Sync {
    /// Gets the name of the iface.
    ///
    /// In Linux, the name is usually the driver name followed by a unit number.
    fn name(&self) -> &str;

    /// Transmits or receives packets queued in the iface, and updates socket status accordingly.
    fn poll(&self);
}

impl dyn Iface {
    /// Binds a socket to the iface.
    ///
    /// After binding the socket to the iface, the iface will handle all packets to and from the
    /// socket.
    ///
    /// If [`BindPortConfig::Ephemeral`] is specified, the iface will pick up an ephemeral port for
    /// the socket.
    ///
    /// FIXME: The reason for binding the socket and the iface together is because there are
    /// limitations inside smoltcp. See discussion at
    /// <https://github.com/smoltcp-rs/smoltcp/issues/779>.
    pub fn bind_socket(
        self: &Arc<Self>,
        socket: Box<AnyUnboundSocket>,
        config: BindPortConfig,
    ) -> core::result::Result<AnyBoundSocket, (Error, Box<AnyUnboundSocket>)> {
        let common = self.common();
        common.bind_socket(self.clone(), socket, config)
    }

    /// Gets the IPv4 address of the iface, if any.
    ///
    /// FIXME: One iface may have multiple IPv4 addresses.
    pub fn ipv4_addr(&self) -> Option<Ipv4Address> {
        self.common().ipv4_addr()
    }

    /// Gets the wait queue that the background polling thread will sleep on.
    fn polling_wait_queue(&self) -> &WaitQueue {
        self.common().polling_wait_queue()
    }

    /// Gets the time when we should perform another poll.
    fn next_poll_at_ms(&self) -> Option<u64> {
        self.common().next_poll_at_ms()
    }
}

mod internal {
    use super::*;

    /// An internal trait that abstracts the common part of different ifaces.
    pub trait IfaceInternal {
        fn common(&self) -> &IfaceCommon;
    }
}
