// SPDX-License-Identifier: MPL-2.0

use alloc::{boxed::Box, sync::Arc};

use smoltcp::wire::Ipv4Address;

use super::port::BindPortConfig;
use crate::{
    errors::BindError,
    socket::{BoundTcpSocket, BoundUdpSocket, UnboundTcpSocket, UnboundUdpSocket},
};

/// A network interface.
///
/// A network interface (abbreviated as iface) is a hardware or software component that connects a
/// computer to a network. Network interfaces can be physical components like Ethernet ports or
/// wireless adapters. They can also be virtual interfaces created by software, such as virtual
/// private network (VPN) connections.
pub trait Iface<E>: internal::IfaceInternal<E> + Send + Sync {
    /// Transmits or receives packets queued in the iface, and updates socket status accordingly.
    ///
    /// The `schedule_next_poll` callback is invoked with the time at which the next poll should be
    /// performed, or `None` if no next poll is required. It's up to the caller to determine the
    /// mechanism to ensure that the next poll happens at the right time (e.g. by setting a timer).
    fn raw_poll(&self, schedule_next_poll: &dyn Fn(Option<u64>));
}

impl<E> dyn Iface<E> {
    /// Gets the extension of the iface.
    pub fn ext(&self) -> &E {
        self.common().ext()
    }

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
    pub fn bind_tcp(
        self: &Arc<Self>,
        socket: Box<UnboundTcpSocket>,
        config: BindPortConfig,
    ) -> core::result::Result<BoundTcpSocket<E>, (BindError, Box<UnboundTcpSocket>)> {
        let common = self.common();
        common.bind_tcp(self.clone(), socket, config)
    }

    pub fn bind_udp(
        self: &Arc<Self>,
        socket: Box<UnboundUdpSocket>,
        config: BindPortConfig,
    ) -> core::result::Result<BoundUdpSocket<E>, (BindError, Box<UnboundUdpSocket>)> {
        let common = self.common();
        common.bind_udp(self.clone(), socket, config)
    }

    /// Gets the IPv4 address of the iface, if any.
    ///
    /// FIXME: One iface may have multiple IPv4 addresses.
    pub fn ipv4_addr(&self) -> Option<Ipv4Address> {
        self.common().ipv4_addr()
    }
}

pub(super) mod internal {
    use crate::iface::common::IfaceCommon;

    /// An internal trait that abstracts the common part of different ifaces.
    pub trait IfaceInternal<E> {
        fn common(&self) -> &IfaceCommon<E>;
    }
}
