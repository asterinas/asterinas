use self::common::IfaceCommon;
use crate::prelude::*;
use smoltcp::iface::SocketSet;

mod any_socket;
mod common;
mod loopback;
mod time;
mod util;
mod virtio;

pub use any_socket::{AnyBoundSocket, AnyUnboundSocket, RawTcpSocket, RawUdpSocket};
pub use any_socket::{RECV_BUF_LEN, SEND_BUF_LEN};
pub use loopback::IfaceLoopback;
pub use smoltcp::wire::{EthernetAddress, IpAddress, IpEndpoint, IpListenEndpoint, Ipv4Address};
pub use util::{spawn_background_poll_thread, BindPortConfig};
pub use virtio::IfaceVirtio;

/// Network interface.
///
/// A network interface (abbreviated as iface) is a hardware or software component that connects a device or computer to a network.
/// Network interfaces can be physical components like Ethernet ports or wireless adapters,
/// or they can be virtual interfaces created by software such as virtual private network (VPN) connections.
pub trait Iface: internal::IfaceInternal + Send + Sync {
    /// The iface name. For linux, usually the driver name followed by a unit number.
    fn name(&self) -> &str;

    /// The optional mac address
    fn mac_addr(&self) -> Option<EthernetAddress>;

    /// Transmit packets queued in the iface, and receive packets queued in the iface.
    /// It any event happens, this function will also update socket status.
    fn poll(&self);

    /// Bind a socket to the iface. So the packet for this socket will be dealt with by the interface.
    /// If port is None, the iface will pick up an empheral port for the socket.
    /// FIXME: The reason for binding socket and interface together is because there are limitations inside smoltcp.
    /// See discussion at https://github.com/smoltcp-rs/smoltcp/issues/779.
    fn bind_socket(
        &self,
        socket: Box<AnyUnboundSocket>,
        config: BindPortConfig,
    ) -> core::result::Result<Arc<AnyBoundSocket>, (Error, Box<AnyUnboundSocket>)> {
        let common = self.common();
        let socket_type_inner = socket.socket_family();
        common.bind_socket(self.arc_self(), socket, config)
    }

    /// The optional ipv4 address
    /// FIXME: An interface indeed support multiple addresses
    fn ipv4_addr(&self) -> Option<Ipv4Address> {
        self.common().ipv4_addr()
    }

    /// The netmask.
    /// FIXME: The netmask and IP address should be one-to-one if there are multiple ip address
    fn netmask(&self) -> Option<Ipv4Address> {
        self.common().netmask()
    }
}

mod internal {
    use super::*;

    /// A helper trait
    pub trait IfaceInternal {
        fn common(&self) -> &IfaceCommon;
        /// The inner socket set
        fn sockets(&self) -> SpinLockGuard<SocketSet<'static>> {
            self.common().sockets()
        }
        /// The inner iface.
        fn iface_inner(&self) -> SpinLockGuard<smoltcp::iface::Interface> {
            self.common().interface()
        }
        /// The time we should do another poll.
        fn next_poll_at_ms(&self) -> Option<u64> {
            self.common().next_poll_at_ms()
        }
        fn arc_self(&self) -> Arc<dyn Iface>;
    }
}
