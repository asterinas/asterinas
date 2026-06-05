// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;
use core::ffi::CStr;

use smoltcp::wire::{Ipv4Address, Ipv4Cidr, Ipv6Address};

use super::{BindPortConfig, BoundTcpPort, BoundUdpPort, InterfaceFlags, InterfaceType};
use crate::{errors::BindError, ext::Ext};

/// A network interface.
///
/// A network interface (abbreviated as iface) is a hardware or software component that connects a
/// computer to a network. Network interfaces can be physical components like Ethernet ports or
/// wireless adapters. They can also be virtual interfaces created by software, such as virtual
/// private network (VPN) connections.
pub trait Iface<E>: internal::IfaceInternal<E> + Send + Sync {
    /// Transmits or receives packets queued in the iface, and updates socket status accordingly.
    fn poll(&self);

    /// Returns the maximum transmission unit.
    fn mtu(&self) -> usize;
}

impl<E: Ext> dyn Iface<E> {
    // FIXME: The reason for binding the socket and the iface together is because there are
    // limitations inside smoltcp. See discussion at
    // <https://github.com/smoltcp-rs/smoltcp/issues/779>.

    /// Binds a TCP port to the iface.
    ///
    /// If no specific port is given in [`BindPortConfig`], the iface will pick up an ephemeral
    /// port.
    pub fn bind_tcp(
        self: &Arc<Self>,
        config: BindPortConfig,
    ) -> Result<BoundTcpPort<E>, BindError> {
        let common = self.common();
        common.bind_tcp(self.clone(), config)
    }

    /// Binds a UDP port to the iface.
    ///
    /// If no specific port is given in [`BindPortConfig`], the iface will pick up an ephemeral
    /// port.
    pub fn bind_udp(
        self: &Arc<Self>,
        config: BindPortConfig,
    ) -> Result<BoundUdpPort<E>, BindError> {
        let common = self.common();
        common.bind_udp(self.clone(), config)
    }

    /// Returns the interface index.
    pub fn index(&self) -> u32 {
        self.common().index()
    }

    /// Gets the name of the iface.
    ///
    /// In Linux, the name is usually the driver name followed by a unit number.
    pub fn name(&self) -> &CStr {
        self.common().name()
    }

    /// Returns the interface type.
    pub fn type_(&self) -> InterfaceType {
        self.common().type_()
    }

    /// Returns the interface flags.
    pub fn flags(&self) -> InterfaceFlags {
        self.common().flags()
    }

    /// Gets the IPv4 address of the iface, if any.
    //
    // FIXME: One iface may have multiple IPv4 addresses.
    pub fn ipv4_addr(&self) -> Option<Ipv4Address> {
        self.common().ipv4_addr()
    }

    /// Gets the IPv6 address of the iface, if any.
    pub fn ipv6_addr(&self) -> Option<Ipv6Address> {
        self.common().ipv6_addr()
    }

    /// Retrieves the prefix length of the interface's IPv4 address.
    ///
    /// Both `Self::ipv4_addr` and this method will either return `Some(_)`
    /// or both will return `None`.
    pub fn prefix_len(&self) -> Option<u8> {
        self.common().prefix_len()
    }

    /// Gets the broadcast address of the iface, if any.
    pub fn broadcast_addr(&self) -> Option<Ipv4Address> {
        let cidr = {
            let common = self.common();
            let ipv4_addr = common.ipv4_addr()?;
            let prefix_len = common.prefix_len()?;
            Ipv4Cidr::new(ipv4_addr, prefix_len)
        };
        cidr.broadcast()
    }

    /// Returns a reference to the associated [`ScheduleNextPoll`].
    ///
    /// [`ScheduleNextPoll`]: crate::iface::sched::ScheduleNextPoll
    pub fn sched_poll(&self) -> &E::ScheduleNextPoll {
        self.common().sched_poll()
    }
}

pub(super) mod internal {
    use crate::{ext::Ext, iface::common::IfaceCommon};

    /// An internal trait that abstracts the common part of different ifaces.
    pub trait IfaceInternal<E> {
        fn common(&self) -> &IfaceCommon<E>
        where
            E: Ext;
    }
}
