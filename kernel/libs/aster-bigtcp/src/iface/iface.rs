// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;

use smoltcp::wire::Ipv4Address;

use super::{port::BindPortConfig, BoundPort, Ext};
use crate::errors::BindError;

/// A network interface.
///
/// A network interface (abbreviated as iface) is a hardware or software component that connects a
/// computer to a network. Network interfaces can be physical components like Ethernet ports or
/// wireless adapters. They can also be virtual interfaces created by software, such as virtual
/// private network (VPN) connections.
pub trait Iface<E: Ext>: internal::IfaceInternal<E> + Send + Sync {
    /// Transmits or receives packets queued in the iface, and updates socket status accordingly.
    fn poll(&self);
}

impl<E: Ext> dyn Iface<E> {
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
    pub fn bind(
        self: &Arc<Self>,
        config: BindPortConfig,
    ) -> core::result::Result<BoundPort<E>, BindError> {
        let common = self.common();
        common.bind(self.clone(), config)
    }

    /// Gets the IPv4 address of the iface, if any.
    ///
    /// FIXME: One iface may have multiple IPv4 addresses.
    pub fn ipv4_addr(&self) -> Option<Ipv4Address> {
        self.common().ipv4_addr()
    }
}

pub(super) mod internal {
    use crate::iface::{common::IfaceCommon, Ext};

    /// An internal trait that abstracts the common part of different ifaces.
    pub trait IfaceInternal<E: Ext> {
        fn common(&self) -> &IfaceCommon<E>;
    }
}
