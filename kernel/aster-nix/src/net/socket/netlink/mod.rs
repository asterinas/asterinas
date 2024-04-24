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
pub use family::NetlinkFamilyType;
pub use multicast_group::GroupIdSet;

use self::{bound::BoundNetlink, family::NETLINK_FAMILIES, unbound::UnboundNetlink};
use super::{SendRecvFlags, Socket, SocketAddr};
use crate::{
    events::{IoEvents, Observer},
    fs::file_handle::FileLike,
    prelude::*,
    process::signal::{CanPoll, Poller},
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
    // FIXME: the `Arc` is used here since we don't want to hold lock
    // when calling `sendto` and recvfrom.
    // Maybe we can think up a clever way to avoid using this `Arc`.
    Bound(Arc<BoundNetlink>),
}

impl Inner {
    fn do_bind(&mut self, addr: NetlinkSocketAddr) -> Result<()> {
        let Inner::Unbound(unbound) = self else {
            return_errno_with_message!(Errno::EINVAL, "the socket is already bound");
        };

        let family_id = unbound.family_id();
        let is_nonblocking = unbound.is_nonblocking();

        let (sender, receiver) = receiver::new_pair(is_nonblocking)?;
        NETLINK_FAMILIES.bind(family_id, &addr, sender)?;

        let bound = Arc::new(BoundNetlink::new(family_id, addr, receiver));
        *self = Inner::Bound(bound);

        Ok(())
    }

    fn is_bound(&self) -> bool {
        matches!(self, Inner::Bound(..))
    }

    fn is_unbound(&self) -> bool {
        matches!(self, Inner::Unbound(..))
    }

    fn do_bind_unspecified(&mut self) -> Result<()> {
        let Inner::Unbound(unbound) = self else {
            return_errno_with_message!(Errno::EINVAL, "the socket is already bound");
        };

        let unspecified_addr = NetlinkSocketAddr::new_unspecified();

        self.do_bind(unspecified_addr)
    }

    fn do_connect(&mut self, remote: NetlinkSocketAddr) -> Result<()> {
        if self.is_unbound() {
            self.do_bind_unspecified()?;
        }

        let Inner::Bound(bound) = self else {
            unreachable!("the socket should always be bound");
        };

        bound.set_remote(remote);

        Ok(())
    }

    fn remote(&self) -> Option<NetlinkSocketAddr> {
        match self {
            Inner::Unbound(unbound) => None,
            Inner::Bound(bound) => bound.remote(),
        }
    }
}

impl NetlinkSocket {
    pub fn new(is_nonblocking: bool, family_type: NetlinkFamilyType) -> Self {
        let family_id = family_type.family_id();
        let unbound = UnboundNetlink::new(is_nonblocking, family_id);
        Self {
            inner: RwMutex::new(Inner::Unbound(unbound)),
        }
    }
}

impl FileLike for NetlinkSocket {
    fn read(&self, buf: &mut [u8]) -> Result<usize> {
        self.recvfrom(buf, SendRecvFlags::empty())
            .map(|(len, _)| len)
    }

    fn write(&self, buf: &[u8]) -> Result<usize> {
        self.sendto(buf, None, SendRecvFlags::empty())
    }

    fn poll(&self, mask: IoEvents, poller: Option<&Poller>) -> IoEvents {
        match &*self.inner.read() {
            Inner::Unbound(unbound) => unbound.poll(mask, poller),
            Inner::Bound(bound) => bound.poll(mask, poller),
        }
    }

    fn register_observer(
        &self,
        observer: Weak<dyn Observer<IoEvents>>,
        mask: IoEvents,
    ) -> Result<()> {
        match &*self.inner.read() {
            Inner::Unbound(unbound) => unbound.register_observer(observer, mask),
            Inner::Bound(bound) => bound.register_observer(observer, mask),
        }

        Ok(())
    }

    fn unregister_observer(
        &self,
        observer: &Weak<dyn Observer<IoEvents>>,
    ) -> Result<Weak<dyn Observer<IoEvents>>> {
        let observer = match &*self.inner.read() {
            Inner::Unbound(unbound) => unbound.unregister_observer(observer),
            Inner::Bound(bound) => bound.unregister_observer(observer),
        };

        // May be refactored after PR #771
        observer.ok_or_else(|| Error::with_message(Errno::ENOENT, "the observer is not registered"))
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
            Inner::Unbound(unbound_socket) => NetlinkSocketAddr::new_unspecified(),
            Inner::Bound(bound_socket) => *bound_socket.addr(),
        };

        Ok(SocketAddr::Netlink(netlink_addr))
    }

    fn peer_addr(&self) -> Result<SocketAddr> {
        self.inner
            .read()
            .remote()
            .map(SocketAddr::Netlink)
            .ok_or_else(|| Error::with_message(Errno::ENOTCONN, "the socket is not connected"))
    }

    fn recvfrom(&self, buf: &mut [u8], flags: SendRecvFlags) -> Result<(usize, SocketAddr)> {
        let bound_netlink = match &*self.inner.read() {
            Inner::Unbound(_) => {
                return_errno_with_message!(Errno::EADDRNOTAVAIL, "the netlink socket is not bound")
            }
            Inner::Bound(bound) => bound.clone(),
        };

        bound_netlink
            .recvfrom(buf, flags)
            .map(|(len, addr)| (len, SocketAddr::Netlink(addr)))
    }

    fn sendto(
        &self,
        buf: &[u8],
        remote: Option<SocketAddr>,
        flags: SendRecvFlags,
    ) -> Result<usize> {
        let remote = if let Some(addr) = remote {
            let SocketAddr::Netlink(netlink_addr) = addr else {
                return_errno_with_message!(Errno::EINVAL, "invalid socket address");
            };
            Some(netlink_addr)
        } else {
            None
        };

        // Ensure `self` is bound
        let inner = self.inner.upread();
        if inner.is_unbound() {
            let mut inner = inner.upgrade();
            inner.do_bind_unspecified()?;
        } else {
            drop(inner);
        };

        let bound_netlink = match &*self.inner.read() {
            Inner::Bound(bound_netlink) => bound_netlink.clone(),
            Inner::Unbound(_) => unreachable!("the socket should always be bound"),
        };

        bound_netlink.sendto(remote, buf, flags)
    }
}

pub fn init() {
    family::init();
}
