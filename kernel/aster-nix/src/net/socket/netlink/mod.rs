// SPDX-License-Identifier: MPL-2.0

//! This module defines netlink socket.
//!
//! Netlink provides a standard socket-based user interface,
//! typically used for communication between user space and kernel space.
//! It can also be used for communication between two user processes.
//!
//! Each Netlink socket belongs to a Netlink family,
//! identified by a family ID (u32).
//! Families are usually defined based on specific functionality.
//! For example, the NETLINK_ROUTE family is used to retrieve or modify routing table entries.
//! Only sockets belonging to the same family can communicate with each other.
//! Some families are pre-defined by the kernel and have fixed purposes,
//! such as NETLINK_ROUTE.
//! Users can also define their own custom families by providing a new family ID.
//!
//! Before communication,
//! a netlink socket needs to be bound to an address,
//! which consists of a port number and a multicast group number.
//!
//! The port number is used for unit cast communication,
//! while the multicast group number is used for multicast communication.
//!
//! For unicast communication, within each family,
//! each port number can only be bound to one socket.
//! However, different families can use the same port number.
//! Typically, the port number is the PID (process ID) of the current process.
//!
//! Multicast allows a message to be sent to one or multiple multicast groups at once.
//! Each family supports up to 32 multicast groups,
//! and each socket can belong to zero or multiple multicast groups.
//!
//! The communication in Netlink is similar to UDP,
//! as it does not require establishing a connection before sending messages.
//! The destination address needs to be specified when sending a message.
//!

pub use addr::NetlinkSocketAddr;
use aster_frame::sync::RwMutex;

use self::{
    addr::FamilyId, bound::BoundNetlink, family::NETLINK_FAMILIES, unbound::UnboundNetlink,
};
use super::{options::SocketOption, SendRecvFlags, Socket, SocketAddr};
use crate::{
    events::IoEvents, fs::file_handle::FileLike, net::socket::netlink::multicast_group::GroupIdSet,
    prelude::*,
};

mod addr;
mod bound;
mod family;
mod multicast_group;
mod receiver;
mod sender;
mod unbound;

/// A netlink socket.
pub struct NetlinkSocket {
    inner: RwMutex<Inner>,
}

enum Inner {
    Unbound(UnboundNetlink),
    Bound(BoundNetlink),
}

impl Inner {
    fn do_bind(&mut self, addr: NetlinkSocketAddr) -> Result<()> {
        let Inner::Unbound(unbound) = self else {
            return_errno_with_message!(Errno::EINVAL, "the socket is already bound");
        };

        let is_nonblocking = unbound.is_nonblocking();

        let (sender, receiver) = receiver::new_pair(is_nonblocking)?;
        NETLINK_FAMILIES.bind(&netlink_addr, sender)?;

        let bound = BoundNetlink::new(netlink_addr, receiver);
        *self = Inner::Bound(bound);

        Ok(())
    }

    fn is_bound(&self) -> bool {
        matches!(self, Inner::Bound(..))
    }

    fn do_connect(&mut self, remote: NetlinkSocketAddr) -> Result<()> {
        if let Inner::Unbound(unbound) = self {
            let default_addr = {
                let family_id = unbound.family_id();
                let port = current!().pid();
                NetlinkSocketAddr::new(family_id, port, GroupIdSet::new_empty())
            };

            self.do_bind(default_addr)?;
        }

        let Inner::Bound(bound) = self else {
            unreachable!("the socket should always be bound");
        };

        bound.set_remote(remote);

        Ok(())
    }

    fn remote(&self) -> Option<&NetlinkSocketAddr> {
        match self {
            Inner::Unbound(unbound) => None,
            Inner::Bound(bound) => bound.remote(),
        }
    }
}

impl NetlinkSocket {
    pub fn new(is_nonblocking: bool, family: FamilyId) -> Self {
        let unbound = UnboundNetlink::new(is_nonblocking, family);
        Self {
            inner: RwMutex::new(Inner::Unbound(unbound)),
        }
    }
}

impl FileLike for NetlinkSocket {
    fn read(&self, buf: &mut [u8]) -> crate::Result<usize> {
        return_errno_with_message!(Errno::EINVAL, "read is not supported");
    }

    fn write(&self, buf: &[u8]) -> crate::Result<usize> {
        return_errno_with_message!(Errno::EINVAL, "write is not supported");
    }

    fn poll(
        &self,
        _mask: IoEvents,
        _poller: Option<&crate::process::signal::Poller>,
    ) -> crate::events::IoEvents {
        crate::events::IoEvents::empty()
    }

    fn register_observer(
        &self,
        observer: alloc::sync::Weak<dyn crate::events::Observer<crate::events::IoEvents>>,
        mask: crate::events::IoEvents,
    ) -> crate::Result<()> {
        return_errno_with_message!(Errno::EINVAL, "register_observer is not supported")
    }

    fn unregister_observer(
        &self,
        observer: &alloc::sync::Weak<dyn crate::events::Observer<crate::events::IoEvents>>,
    ) -> crate::Result<alloc::sync::Weak<dyn crate::events::Observer<crate::events::IoEvents>>>
    {
        return_errno_with_message!(Errno::EINVAL, "unregister_observer is not supported")
    }

    fn as_socket(self: alloc::sync::Arc<Self>) -> Option<alloc::sync::Arc<dyn Socket>> {
        None
    }

    fn as_device(&self) -> Option<alloc::sync::Arc<dyn crate::fs::device::Device>> {
        None
    }
}

impl Socket for NetlinkSocket {
    fn bind(&self, socket_addr: SocketAddr) -> Result<()> {
        let SocketAddr::Netlink(netlink_addr) = socket_addr else {
            return_errno_with_message!(Errno::EAFNOSUPPORT, "the address is invalid");
        };

        let mut inner = self.inner.write();
        inner.do_bind(netlink_addr)?;

        Ok(())
    }

    fn connect(&self, socket_addr: SocketAddr) -> Result<()> {
        let SocketAddr::Netlink(netlink_addr) = socket_addr else {
            return_errno_with_message!(Errno::EAFNOSUPPORT, "the address is invalie");
        };

        let mut inner = self.inner.write();
        inner.do_connect(netlink_addr)
    }

    fn addr(&self) -> Result<SocketAddr> {
        let inner = self.inner.read();
        let netlink_addr = match &*self.inner.read() {
            Inner::Unbound(unbound_socket) => {
                NetlinkSocketAddr::new_unspecified(unbound_socket.family_id())
            }
            Inner::Bound(bound_socket) => bound_socket.addr().clone(),
        };

        Ok(SocketAddr::Netlink(netlink_addr))
    }

    fn peer_addr(&self) -> Result<SocketAddr> {
        self.inner
            .read()
            .remote()
            .map(Clone::clone)
            .ok_or_else(|| Error::with_message(Errno::ENOTCONN, "the socket is not connected"))
    }

    fn recvfrom(&self, buf: &mut [u8], flags: SendRecvFlags) -> Result<(usize, SocketAddr)> {
        return_errno_with_message!(Errno::EOPNOTSUPP, "recvfrom() is not supported");
    }

    fn sendto(
        &self,
        buf: &[u8],
        remote: Option<SocketAddr>,
        flags: SendRecvFlags,
    ) -> Result<usize> {
        return_errno_with_message!(Errno::EOPNOTSUPP, "recvfrom() is not supported");
    }
}

pub fn init() {
    family::init();
}
